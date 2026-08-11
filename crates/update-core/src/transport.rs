// HTTP 传输层（对应原 Go internal/transport/http.go）。
//
// 基于 ureq（阻塞 HTTP/1.1 + rustls TLS），处理认证、自定义头、超时、
// 代理（环境变量）、TLS 跳过校验，并把网络/HTTP 失败归一化为带分类的
// Error，供 CLI 映射退出码。
use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use sha2::{Digest, Sha256};

/// 失败分类（供 CLI 退出码映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// 源错误：网络、HTTP 状态或上游解析失败。
    Source,
    /// 下载错误：文件下载/校验和/写盘失败。
    Download,
}

/// 归一化的传输失败。
#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub status_code: Option<u16>,
}

impl Error {
    pub fn source(msg: String) -> Self {
        Self {
            kind: ErrorKind::Source,
            message: msg,
            status_code: None,
        }
    }
    pub fn download(msg: String) -> Self {
        Self {
            kind: ErrorKind::Download,
            message: msg,
            status_code: None,
        }
    }
    pub fn with_status(status: u16, msg: String) -> Self {
        Self {
            kind: ErrorKind::Source,
            message: msg,
            status_code: Some(status),
        }
    }
    pub fn is_status(&self, code: u16) -> bool {
        self.status_code == Some(code)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// 请求凭据。
#[derive(Debug, Clone, Default)]
pub struct Auth {
    /// ""、 "bearer" 或 "basic"
    pub ty: String,
    pub token: String,
    pub username: String,
}

/// Client 配置。
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub timeout: Option<Duration>,
    pub headers: HashMap<String, String>,
    pub auth: Option<Auth>,
    /// 跳过 TLS 校验；仅用于私有网络自签名证书的自定义源。
    pub insecure: bool,
}

pub const MAX_BODY_BYTES: usize = 32 << 20; // 32 MiB，与 Go 版一致
const MAX_ERROR_BODY_BYTES: usize = 512;

/// 共享 HTTP 客户端。
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    headers: HashMap<String, String>,
    auth: Option<Auth>,
}

/// 校验失败缓存证书的 TLS 验证器（仅 insecure 模式）。
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
            .to_vec()
    }
}

fn insecure_tls_config() -> Arc<rustls::ClientConfig> {
    let provider = rustls::crypto::ring::default_provider();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .expect("supported protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();
    Arc::new(config)
}

impl Client {
    /// 构建 Client。超时默认 30s。
    pub fn new(opts: Options) -> Self {
        let mut builder = ureq::AgentBuilder::new()
            .timeout(opts.timeout.unwrap_or(Duration::from_secs(30)))
            .redirects(10); // 与 Go net/http 默认一致
        if opts.insecure {
            // 显式 opt-in：私有网络自签名证书
            builder = builder.tls_config(insecure_tls_config());
        }
        Client {
            agent: builder.build(),
            headers: opts.headers,
            auth: opts.auth,
        }
    }

    fn apply_auth(&self, mut req: ureq::Request) -> ureq::Request {
        if let Some(a) = &self.auth {
            match a.ty.as_str() {
                "bearer" => {
                    if !a.token.is_empty() {
                        req = req.set("Authorization", &format!("Bearer {}", a.token));
                    }
                }
                "basic" => {
                    let encoded = base64::engine::general_purpose::STANDARD
                        .encode(format!("{}:{}", a.username, a.token));
                    req = req.set("Authorization", &format!("Basic {encoded}"));
                }
                _ => {}
            }
        }
        req
    }

    /// 执行 GET，2xx 返回可读的响应；其余返回归一化 Error。
    /// 2xx 响应体必须被读取（drop 后连接不归还连接池）。
    pub fn get(&self, url: &str) -> Result<ureq::Response, Error> {
        let mut req = self.agent.get(url);
        for (k, v) in &self.headers {
            req = req.set(k, v);
        }
        req = self.apply_auth(req);
        match req.call() {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(code, resp)) => {
                let body = read_error_body(resp);
                Err(Error::with_status(
                    code,
                    format!("request {url}: status {code}: {body}"),
                ))
            }
            Err(ureq::Error::Transport(t)) => {
                if t.kind() == ureq::ErrorKind::InvalidUrl {
                    return Err(Error::source(format!("invalid url {url:?}: {t}")));
                }
                Err(Error::source(format!("request {url}: {t}")))
            }
        }
    }

    /// 获取并解析 JSON；响应体上限 32 MiB。
    pub fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, Error> {
        let resp = self.get(url)?;
        let mut data = Vec::new();
        resp.into_reader()
            .take((MAX_BODY_BYTES + 1) as u64)
            .read_to_end(&mut data)
            .map_err(|e| Error::source(format!("read response from {url}: {e}")))?;
        if data.len() > MAX_BODY_BYTES {
            return Err(Error::source(format!(
                "decode response from {url}: response body exceeds 32 MiB"
            )));
        }
        serde_json::from_slice(&data)
            .map_err(|e| Error::source(format!("decode response from {url}: {e}")))
    }

    /// 流式下载 raw_url 到 dest。先写同目录临时文件、成功时原子重命名，
    /// 半成品永远不会留在目标位置。expect_sha256 非空时校验，不匹配则
    /// 删除临时文件并报错。
    pub fn download(&self, url: &str, dest: &str, expect_sha256: &str) -> Result<(), Error> {
        let resp = self.get(url)?;
        let mut reader = resp.into_reader();

        let dir = match Path::new(dest).parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => Path::new(".").to_path_buf(),
        };
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::download(format!("create dir {}: {e}", dir.display())))?;

        let tmp = temp_path(&dir, dest);
        let mut tmp_file = match std::fs::File::create(&tmp) {
            Ok(f) => f,
            Err(e) => return Err(Error::download(format!("create temp file: {e}"))),
        };

        let mut hasher = Sha256::new();
        let copy_res = {
            let mut w = HashingWriter::new(&mut tmp_file, &mut hasher);
            std::io::copy(&mut reader, &mut w)
        };
        if let Err(e) = copy_res {
            let _ = std::fs::remove_file(&tmp);
            return Err(Error::download(format!("download {url}: {e}")));
        }
        if let Err(e) = tmp_file.sync_all() {
            let _ = std::fs::remove_file(&tmp);
            return Err(Error::download(format!("sync temp file: {e}")));
        }
        drop(tmp_file);

        let got = hex_encode(&hasher.finalize());
        let expect = expect_sha256
            .trim()
            .trim_start_matches("sha256:")
            .trim_start_matches("SHA256:")
            .to_lowercase();
        if !expect.is_empty() && expect != got {
            let _ = std::fs::remove_file(&tmp);
            return Err(Error::download(format!(
                "checksum mismatch for {url}: expected sha256:{expect}, got sha256:{got}"
            )));
        }
        std::fs::rename(&tmp, dest)
            .map_err(|e| Error::download(format!("rename to {dest}: {e}")))?;
        Ok(())
    }
}

/// 边写文件边更新哈希的 Write 适配器。
struct HashingWriter<'a, W: Write> {
    inner: &'a mut W,
    hasher: &'a mut Sha256,
}

impl<'a, W: Write> HashingWriter<'a, W> {
    fn new(inner: &'a mut W, hasher: &'a mut Sha256) -> Self {
        HashingWriter { inner, hasher }
    }
}

impl<W: Write> Write for HashingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// hex 小写编码（Go hex.EncodeToString 语义）。
pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 读取错误响应体（≤512 字节，去掉首尾空白）。
fn read_error_body(resp: ureq::Response) -> String {
    let mut buf = Vec::new();
    let mut reader = resp.into_reader().take(MAX_ERROR_BODY_BYTES as u64 + 1);
    let _ = reader.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).trim().to_string()
}

/// 生成同目录临时文件路径：".<name>.tmp-<pid>-<time>"
fn temp_path(dir: &Path, dest: &str) -> std::path::PathBuf {
    let base = Path::new(dest)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    dir.join(format!(".{base}.tmp-{}-{}", std::process::id(), nanos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{json_response, req_header, RecordingServer, TestServer};

    fn sha256_hex(b: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(b);
        hex_encode(&h.finalize())
    }

    #[test]
    fn test_get_headers_and_auth() {
        let srv = TestServer::new(|req| {
            let auth = req_header(req, "Authorization");
            let custom = req_header(req, "X-Custom");
            assert_eq!(auth, "Bearer tok");
            assert_eq!(custom, "v1");
            crate::httpd::Response::text(200, "text/plain", "ok")
        });
        let c = Client::new(Options {
            headers: HashMap::from([("X-Custom".to_string(), "v1".to_string())]),
            auth: Some(Auth {
                ty: "bearer".to_string(),
                token: "tok".to_string(),
                username: String::new(),
            }),
            ..Default::default()
        });
        let resp = c.get(&srv.url("/")).unwrap();
        let mut body = String::new();
        resp.into_reader().read_to_string(&mut body).unwrap();
        assert_eq!(body, "ok");
    }

    #[test]
    fn test_get_basic_auth() {
        let got = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let got2 = got.clone();
        let srv = TestServer::new(move |req| {
            *got2.lock().unwrap() = req_header(req, "Authorization");
            crate::httpd::Response::text(200, "text/plain", "ok")
        });
        let c = Client::new(Options {
            auth: Some(Auth {
                ty: "basic".to_string(),
                username: "bob".to_string(),
                token: "pw".to_string(),
            }),
            ..Default::default()
        });
        let resp = c.get(&srv.url("/")).unwrap();
        let mut body = String::new();
        resp.into_reader().read_to_string(&mut body).unwrap();
        assert_eq!(body, "ok");
        assert!(got.lock().unwrap().starts_with("Basic "));
        // 解码验证凭据
        let enc = got.lock().unwrap().trim_start_matches("Basic ").to_string();
        let dec = base64::engine::general_purpose::STANDARD
            .decode(enc)
            .unwrap();
        assert_eq!(String::from_utf8(dec).unwrap(), "bob:pw");
    }

    #[test]
    fn test_get_http_error() {
        let srv =
            TestServer::new(|_| crate::httpd::Response::json(404, r#"{"message":"Not Found"}"#));
        let c = Client::new(Options::default());
        let err = c.get(&srv.url("/")).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Source);
        assert_eq!(err.status_code, Some(404));
        assert!(
            err.message.contains("Not Found"),
            "message = {}",
            err.message
        );
    }

    #[test]
    fn test_get_json() {
        #[derive(serde::Deserialize)]
        struct Out {
            a: i64,
        }
        let srv = TestServer::new(|_| json_response(200, r#"{"a":1}"#));
        let c = Client::new(Options::default());
        let out: Out = c.get_json(&srv.url("/")).unwrap();
        assert_eq!(out.a, 1);
    }

    #[test]
    fn test_download_checksum_and_atomic() {
        let content = b"hello update";
        let sum = sha256_hex(content);
        let srv = TestServer::new(|_| {
            crate::httpd::Response::text(200, "application/octet-stream", content.to_vec())
        });
        let c = Client::new(Options::default());

        let dir = std::env::temp_dir().join(format!("update-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("app.bin");
        let dests = dest.to_str().unwrap();

        // 校验和不匹配 → 报错且不留文件（包括不留临时文件）
        let err = c
            .download(&srv.url("/"), dests, &format!("sha256:{}", "0".repeat(64)))
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Download, "err = {err}");
        assert!(!dest.exists(), "dest should not exist after mismatch");
        let leftover: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(leftover.is_empty(), "leftover temp files: {leftover:?}");

        // 校验和正确 → 文件创建
        c.download(&srv.url("/"), dests, &format!("sha256:{sum}"))
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), content);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_download_network_error_kind() {
        // 连接失败应归类为源错误（下载前的请求阶段）
        let c = Client::new(Options::default());
        let err = c
            .download(
                "http://127.0.0.1:1/file",
                "/tmp/opencode/never-created.bin",
                "",
            )
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Source, "err = {err}");
    }

    #[test]
    fn test_invalid_url() {
        let c = Client::new(Options::default());
        let err = c.get("not a url").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Source);
        assert!(
            err.message.contains("invalid url"),
            "message = {}",
            err.message
        );
    }

    #[test]
    fn test_download_wrong_dir_creates() {
        // 目标目录不存在时应自动创建（Go 版行为）
        let content = b"x";
        let srv = TestServer::new(|_| {
            crate::httpd::Response::text(200, "application/octet-stream", content.to_vec())
        });
        let c = Client::new(Options::default());
        let dest =
            std::env::temp_dir().join(format!("update-dl-{}/sub/out.bin", std::process::id()));
        let dests = dest.to_str().unwrap();
        c.download(&srv.url("/"), dests, "").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), content);
        let _ = std::fs::remove_dir_all(dest.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn test_error_body_truncated_to_512() {
        let big = "x".repeat(2000);
        let srv = TestServer::new(move |_| crate::httpd::Response::json(500, big.clone()));
        let c = Client::new(Options::default());
        let err = c.get(&srv.url("/")).unwrap_err();
        assert!(
            err.message.len() < 600,
            "error message not truncated: {}",
            err.message.len()
        );
        assert!(err.message.contains("status 500"));
    }

    #[test]
    fn test_recording_server() {
        let rec = RecordingServer::new(|_, _| json_response(200, r#"{"ok":1}"#));
        let c = Client::new(Options::default());
        let _ = c.get(&rec.server.url("/a?x=1")).unwrap();
        let reqs = rec.requests.lock().unwrap();
        assert_eq!(reqs[0].0, "/a");
        let path = &reqs[0].1;
        let host = path
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("host"))
            .unwrap()
            .1
            .clone();
        assert!(host.starts_with("127.0.0.1:"));
    }
}
