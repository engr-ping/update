// 最小 HTTP/1.1 服务器（纯 std 实现，替代 Go net/http 的服务端部分）。
//
// 面向本工程的三个使用场景：updateserver 静态分发、客户端测试的模拟源。
// 特性：
//   - GET / HEAD（其余方法 405）
//   - 始终带 Content-Length 的响应，支持 keep-alive 复用
//   - 请求行 ≤ 8 KiB、单头 ≤ 16 KiB、总头 ≤ 64 KiB
//   - 路径按段百分号解码，便于路由与穿越防护
//   - Body::File 支持流式发送大文件
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

pub const MAX_REQUEST_LINE_BYTES: usize = 8192;
pub const MAX_HEADER_LINE_BYTES: usize = 16384;
pub const MAX_HEADERS_BYTES: usize = 65536;

/// 已解析的 HTTP 请求。
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// 请求目标（不含 query）
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    /// 是否 TLS 连接（updateserver 不原生支持 TLS，恒为 false）
    pub tls: bool,
}

impl Request {
    /// 大小写不敏感地读取请求头。
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase() == name)
            .map(|(_, v)| v.as_str())
    }

    /// 客户端 Host（不含端口规范化）。
    pub fn host(&self) -> String {
        self.header("host").unwrap_or_default().to_string()
    }
}

/// 响应体。
pub enum Body {
    None,
    Bytes(Vec<u8>),
    /// 流式发送文件
    File(std::fs::File, u64),
}

/// HTTP 响应。
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Response {
    pub fn new(status: u16) -> Self {
        Response {
            status,
            headers: Vec::new(),
            body: Body::None,
        }
    }
    pub fn text(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Response {
            status,
            headers: vec![("Content-Type".to_string(), content_type.to_string())],
            body: Body::Bytes(body.into()),
        }
    }
    pub fn json(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::text(status, "application/json", body)
    }
}

pub type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;

const STATUS_TEXT: &[(u16, &str)] = &[
    (200, "OK"),
    (400, "Bad Request"),
    (404, "Not Found"),
    (405, "Method Not Allowed"),
    (500, "Internal Server Error"),
];

fn status_text(code: u16) -> &'static str {
    STATUS_TEXT
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, s)| *s)
        .unwrap_or("OK")
}

/// 在 listener 上提供服务，每个连接一个线程（连接内支持 keep-alive）。
/// 返回后仅当 accept 失败（一般是被关闭）。
pub fn serve(listener: TcpListener, handler: Handler) -> std::io::Result<()> {
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let h = handler.clone();
                if let Err(e) = stream.set_nodelay(true) {
                    let _ = e;
                }
                std::thread::spawn(move || handle_conn(stream, h));
            }
            Err(_) => return Ok(()),
        }
    }
    Ok(())
}

fn handle_conn(stream: TcpStream, handler: Handler) {
    let reader_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
    loop {
        let req = match read_request(&mut reader) {
            Ok(Some(r)) => r,
            Ok(None) => break, // EOF
            Err(_) => break,
        };
        // keep_alive=true 表示本请求后继续保持连接
        let keep_alive = !has_connection_close(&req);
        let resp = if req.method == "GET" || req.method == "HEAD" {
            handler(&req)
        } else {
            Response {
                status: 405,
                headers: Vec::new(),
                body: Body::None,
            }
        };
        if write_response(&mut writer, &req, resp, keep_alive).is_err() || !keep_alive {
            break;
        }
    }
}

fn has_connection_close(req: &Request) -> bool {
    req.header("connection")
        .map(|v| {
            v.to_ascii_lowercase()
                .split(',')
                .any(|t| t.trim() == "close")
        })
        .unwrap_or(false)
}

/// 读取一个请求。返回 Ok(None) 表示连接关闭。
fn read_request(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<Request>> {
    // 请求行
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Ok(None);
    }
    if line.len() > MAX_REQUEST_LINE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request line too long",
        ));
    }
    let line = line.trim_end_matches(['\r', '\n']);
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_ascii_uppercase();
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if method.is_empty() || target.is_empty() || version != "HTTP/1.1" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad request line",
        ));
    }
    let (path, query) = match target.find('?') {
        Some(i) => (&target[..i], &target[i + 1..]),
        None => (target, ""),
    };

    // 头
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut total = 0usize;
    loop {
        let mut hline = String::new();
        let n = reader.read_line(&mut hline)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "unexpected EOF in headers",
            ));
        }
        if hline.len() > MAX_HEADER_LINE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "header line too long",
            ));
        }
        total += hline.len();
        if total > MAX_HEADERS_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "headers too large",
            ));
        }
        let hline = hline.trim_end_matches(['\r', '\n']);
        if hline.is_empty() {
            break;
        }
        if let Some((k, v)) = hline.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bad header line",
            ));
        }
    }

    // GET/HEAD 无请求体；若客户端声明了 body，丢弃（防御性）。
    if let Some(cl) = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .map(|(_, v)| v.parse::<u64>().unwrap_or(0))
    {
        if cl > 0 {
            let mut skipped = 0u64;
            let mut buf = [0u8; 8192];
            while skipped < cl {
                let want = ((cl - skipped) as usize).min(buf.len());
                let n = reader.read(&mut buf[..want])?;
                if n == 0 {
                    break;
                }
                skipped += n as u64;
            }
        }
    }

    Ok(Some(Request {
        method,
        path: path.to_string(),
        query: query.to_string(),
        headers,
        tls: false,
    }))
}

fn write_response(
    writer: &mut TcpStream,
    req: &Request,
    resp: Response,
    keep_alive: bool,
) -> std::io::Result<()> {
    let (file, file_size) = match &resp.body {
        Body::File(f, size) => (Some(f), Some(*size)),
        Body::Bytes(b) => (None, Some(b.len() as u64)),
        Body::None => (None, None),
    };
    let length = file_size.unwrap_or(0);

    let mut out = String::new();
    out.push_str(&format!(
        "HTTP/1.1 {} {}\r\n",
        resp.status,
        status_text(resp.status)
    ));
    for (k, v) in &resp.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    // 若 handler 未显式设置 Content-Type，按响应体类型补默认
    if !resp
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        if let Body::Bytes(_) = &resp.body {
            out.push_str("Content-Type: application/octet-stream\r\n");
        }
    }
    out.push_str(&format!("Content-Length: {length}\r\n"));
    if keep_alive {
        out.push_str("Connection: keep-alive\r\n");
    } else {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    writer.write_all(out.as_bytes())?;

    let is_head = req.method == "HEAD";
    match (&resp.body, file) {
        (Body::Bytes(b), _) if !b.is_empty() && !is_head => {
            writer.write_all(b)?;
        }
        (Body::File(f, size), Some(_)) if !is_head => {
            let mut reader = f;
            let mut remaining = *size;
            let mut buf = vec![0u8; 64 * 1024];
            while remaining > 0 {
                let want = (remaining as usize).min(buf.len());
                let n = reader.read(&mut buf[..want])?;
                if n == 0 {
                    break;
                }
                writer.write_all(&buf[..n])?;
                remaining -= n as u64;
            }
        }
        _ => {}
    }
    writer.flush()
}

/// 对请求路径按段做百分号解码；解不出 UTF-8 时报错。
#[allow(clippy::result_unit_err)]
pub fn decode_path(path: &str) -> Result<String, ()> {
    let mut out = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = |b: u8| -> Option<u8> {
                    match b {
                        b'0'..=b'9' => Some(b - b'0'),
                        b'a'..=b'f' => Some(b - b'a' + 10),
                        b'A'..=b'F' => Some(b - b'A' + 10),
                        _ => None,
                    }
                };
                let h = hex(bytes[i + 1]).ok_or(())?;
                let l = hex(bytes[i + 2]).ok_or(())?;
                out.push(h << 4 | l);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_path() {
        assert_eq!(decode_path("/feed/a%20b.json").unwrap(), "/feed/a b.json");
        assert_eq!(
            decode_path("/package/my-app/..%2Fx").unwrap(),
            "/package/my-app/../x"
        );
        assert!(decode_path("/%zz").is_err());
    }

    #[test]
    fn test_serve_get_and_404() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handler: Handler = Arc::new(|req| {
            if req.path == "/hi" {
                Response::text(200, "text/plain", "hello")
            } else {
                Response::text(404, "application/json", r#"{"error":"not found"}"#)
            }
        });
        let h = handler.clone();
        let t = std::thread::spawn(move || serve(listener, h));
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(b"GET /hi HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(reply.contains("Content-Length: 5\r\n"));
        assert!(reply.ends_with("\r\n\r\nhello"));
        let _ = t;
    }

    #[test]
    fn test_serve_head_and_method() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handler: Handler = Arc::new(|req| {
            if req.path == "/x" {
                Response::text(200, "text/plain", "body!")
            } else {
                Response::new(404)
            }
        });
        let h = handler.clone();
        std::thread::spawn(move || serve(listener, h));
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(b"HEAD /x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.contains("Content-Length: 5\r\n"));
        assert!(!reply.ends_with("body!"));
    }

    #[test]
    fn test_keep_alive_two_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handler: Handler = Arc::new(|req| {
            let n = req.path.trim_start_matches('/').to_string();
            Response::text(200, "text/plain", n)
        });
        let h = handler.clone();
        std::thread::spawn(move || serve(listener, h));
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(b"GET /one HTTP/1.1\r\nHost: x\r\n\r\nGET /two HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.contains("one"));
        assert!(reply.contains("two"));
    }
}
