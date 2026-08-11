// 测试工具：本地模拟 HTTP 源（对应 Go 版测试里的 net/http/httptest）。
// 构建在 httpd 之上，给 client/source/cli 的测试提供无需真实网络的源。
#![cfg(test)]
use crate::httpd::{self, Handler, Request, Response};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// 快速模拟源：启动一个后台线程的 HTTP 服务器。
pub struct TestServer {
    pub url: String,
    _thread: std::thread::JoinHandle<()>,
}

impl TestServer {
    /// 启动一个每分钟响应都交给 handler 的模拟源。
    pub fn new<F>(handler: F) -> Self
    where
        F: Fn(&Request) -> Response + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let h: Handler = Arc::new(handler);
        let thread = std::thread::spawn(move || {
            let _ = httpd::serve(listener, h);
        });
        TestServer {
            url: format!("http://{addr}"),
            _thread: thread,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.url, path)
    }
}

type RecordedRequest = (String, Vec<(String, String)>);
type RequestLog = Arc<std::sync::Mutex<Vec<RecordedRequest>>>;

/// 可捕获请求的模拟源：所有请求进入共享 Vec。
pub struct RecordingServer {
    pub server: TestServer,
    pub requests: RequestLog,
}

impl RecordingServer {
    /// 每个请求先记录（path, headers），再交给 handler 响应。
    pub fn new<F>(handler: F) -> Self
    where
        F: Fn(&Request, &[(String, String)]) -> Response + Send + Sync + 'static,
    {
        let requests: RequestLog = Arc::new(std::sync::Mutex::new(Vec::new()));
        let r2 = requests.clone();
        let server = TestServer::new(move |req: &Request| {
            let mut rec = r2.lock().unwrap();
            rec.push((req.path.clone(), req.headers.clone()));
            handler(req, &rec[rec.len() - 1].1)
        });
        RecordingServer { server, requests }
    }
}

pub fn json_response(status: u16, body: &str) -> Response {
    Response::json(status, body.as_bytes().to_vec())
}

pub fn text_response(status: u16, body: &str) -> Response {
    Response::text(status, "text/plain", body.as_bytes().to_vec())
}

/// 原始请求 Header 的值。
pub fn req_header(req: &Request, name: &str) -> String {
    req.header(name).unwrap_or_default().to_string()
}

/// 最简单的原始客户端：直接发字节并读回（rugged keep-alive 测试用）。
pub fn raw_request(addr: std::net::SocketAddr, bytes: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(bytes).unwrap();
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).unwrap();
    reply
}

pub fn raw_response_headers(addr: std::net::SocketAddr, bytes: &[u8]) -> String {
    let raw = raw_request(addr, bytes);
    String::from_utf8_lossy(&raw).to_string()
}
