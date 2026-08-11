// 只读分发服务器（对应原 Go server/）。
//
// 数据目录布局：<dir>/package/<name>/<version>/<file>，每个版本目录可选
// meta.json 增强元数据。feed 格式与 custom 源协议完全一致（docs/design.md §6）。
//
// 端点：
//   GET /feed/<name>.json                          该软件的发布清单（新版本在前）
//   GET /feeds.json                                全部软件与其版本列表
//   GET /package/<name>/<version>/<file>           产物下载（路径穿越防护，只读）
//   GET /healthz                                   健康检查
use crate::httpd::{self, Request, Response};
use crate::semver::compare;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PACKAGE_DIR: &str = "package";
const META_FILE: &str = "meta.json";

/// write_error 的格式化变体：werr!(500, "list versions: {e}")
macro_rules! werr {
    ($status:expr, $($arg:tt)+) => {
        write_error($status, &format!($($arg)+))
    };
}

/// 每版本可选元数据文件。除 notes/checksum 外其余字段缺省回退文件系统默认值。
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct MetaFile {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(
        default,
        rename = "published_at",
        skip_serializing_if = "String::is_empty"
    )]
    pub published_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checksum: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<std::collections::HashMap<String, AssetMeta>>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct AssetMeta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub size: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// 与客户端侧统一发布模型镜像（docs/design.md §6），client 可直接消费。
#[derive(Debug, Default, Serialize)]
pub struct Release {
    pub version: String,
    #[serde(rename = "published_at", skip_serializing_if = "String::is_empty")]
    pub published_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub checksum: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Serialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub size: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sha256: String,
}

pub struct Server {
    pub dir: String,
}

impl Server {
    pub fn new(dir: &str) -> Self {
        Server {
            dir: dir.to_string(),
        }
    }

    /// <dir>/package/<name>，防御路径穿越。
    pub fn product_dir(&self, name: &str) -> Result<PathBuf, String> {
        if name.is_empty() || name.contains('\\') {
            return Err(format!("invalid product name {name:?}"));
        }
        let clean = Path::new(name);
        if clean == Path::new(".") || clean.is_absolute() {
            return Err(format!("invalid product name {name:?}"));
        }
        for part in name.split('/') {
            if part == ".." {
                return Err(format!("invalid product name {name:?}"));
            }
        }
        Ok(Path::new(&self.dir).join(PACKAGE_DIR).join(name))
    }

    /// package/ 下的产品名（子目录名，按字典序）。
    fn list_products(&self) -> Result<Vec<String>, String> {
        let base = Path::new(&self.dir).join(PACKAGE_DIR);
        let mut names = Vec::new();
        match std::fs::read_dir(&base) {
            Ok(entries) => {
                for e in entries {
                    let e = e.map_err(|e| e.to_string())?;
                    if e.path().is_dir() {
                        names.push(e.file_name().to_string_lossy().to_string());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(names),
            Err(e) => return Err(e.to_string()),
        }
        names.sort();
        Ok(names)
    }

    /// 产品的版本目录，按 semver 降序（新在前）。
    fn list_versions(&self, name: &str) -> Result<Vec<String>, String> {
        let pdir = self.product_dir(name)?;
        let mut vs = Vec::new();
        match std::fs::read_dir(&pdir) {
            Ok(entries) => {
                for e in entries {
                    let e = e.map_err(|e| e.to_string())?;
                    if e.path().is_dir() {
                        vs.push(e.file_name().to_string_lossy().to_string());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vs),
            Err(e) => return Err(e.to_string()),
        }
        vs.sort_by(|a, b| compare(b, a).cmp(&0));
        Ok(vs)
    }

    /// 读取 <version>/meta.json（不存在返回 None）。
    fn load_meta(&self, pdir: &Path, ver: &str) -> Result<Option<MetaFile>, String> {
        let path = pdir.join(ver).join(META_FILE);
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let m: MetaFile = serde_json::from_slice(&data)
            .map_err(|e| format!("parse meta.json in {}/{}: {e}", pdir.display(), ver))?;
        Ok(Some(m))
    }

    /// 从文件系统组装一个发布；asset_urls 把文件名映射到下载 URL。
    fn build_release(
        &self,
        pdir: &Path,
        ver: &str,
        asset_urls: &std::collections::HashMap<String, String>,
    ) -> Result<Option<Release>, String> {
        let meta = self.load_meta(pdir, ver)?;
        let mut r = Release {
            version: ver.to_string(),
            ..Default::default()
        };
        if let Some(m) = &meta {
            r.name = m.name.clone();
            r.notes = m.notes.clone();
            r.published_at = m.published_at.clone();
            r.checksum = m.checksum.clone();
        }

        let vdir = pdir.join(ver);
        let files: Vec<std::fs::DirEntry> = match std::fs::read_dir(&vdir) {
            Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        for f in &files {
            let fname = f.file_name().to_string_lossy().to_string();
            if f.path().is_dir() || fname == META_FILE {
                continue;
            }
            let size = f.metadata().map(|m| m.len() as i64).unwrap_or(0);
            let mut a = Asset {
                name: fname.clone(),
                url: asset_urls.get(&fname).cloned().unwrap_or_default(),
                size,
                sha256: String::new(),
            };
            if let Some(m) = &meta {
                if let Some(am) = m.assets.as_ref().and_then(|map| map.get(&fname)) {
                    a.sha256 = am.sha256.clone();
                    if am.size > 0 {
                        a.size = am.size;
                    }
                }
            }
            r.assets.push(a);
        }
        if r.published_at.is_empty() && !files.is_empty() {
            if let Ok(meta) = std::fs::metadata(&vdir) {
                if let Ok(mtime) = meta.modified() {
                    if let Ok(secs) = mtime.duration_since(std::time::UNIX_EPOCH) {
                        r.published_at = rfc3339_from_unix(secs.as_secs() as i64);
                    }
                }
            }
        }
        r.assets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Some(r))
    }

    /// 按请求生成产物的绝对下载 URL（支持反代 X-Forwarded-Proto）。
    fn asset_url(&self, req: &Request, name: &str, ver: &str, file: &str) -> String {
        let scheme = if req.header("x-forwarded-proto") == Some("https") || req.tls {
            "https"
        } else {
            "http"
        };
        let host = {
            let h = req.host();
            if h.is_empty() {
                "localhost".to_string()
            } else {
                h
            }
        };
        format!("{scheme}://{host}/{PACKAGE_DIR}/{name}/{ver}/{file}")
    }

    /// 路由：/feed/...、/feeds.json、/package/...、/healthz。
    /// 路径先按段做百分号解码（对齐 Go net/http 的 URL.Path 语义），
    /// 解码后 product_dir / 下载防护在解码结果上做穿越检查。
    pub fn handle(&self, req: &Request) -> Response {
        let path = match crate::httpd::decode_path(&req.path) {
            Ok(p) => p,
            Err(_) => return werr!(400, "invalid escape sequence in path"),
        };
        if path == "/healthz" {
            return Response::json(200, br#"{"status":"ok"}"#.to_vec());
        }
        if let Some(rest) = path.strip_prefix("/feed/") {
            return self.handle_feed_path(req, rest);
        }
        if path == "/feeds.json" {
            return self.handle_feeds();
        }
        if let Some(rest) = path.strip_prefix("/package/") {
            return self.handle_download(rest);
        }
        werr!(404, "not found")
    }

    /// GET /feed/<name>.json（rest 为 "/feed/" 之后的剩余路径）。
    fn handle_feed_path(&self, req: &Request, rest: &str) -> Response {
        if rest.strip_suffix(".json").is_none() || rest.is_empty() {
            return werr!(
                400,
                "invalid feed path {:?} (want /feed/<name>.json)",
                format!("/feed/{rest}")
            );
        }
        let name = &rest[..rest.len() - ".json".len()];
        if name.is_empty() || name.contains('/') {
            return werr!(
                400,
                "invalid feed path {:?} (want /feed/<name>.json)",
                format!("/feed/{rest}")
            );
        }
        let pdir = match self.product_dir(name) {
            Ok(p) => p,
            Err(e) => return werr!(400, "{}", e),
        };
        let vers = match self.list_versions(name) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("updateserver: list versions {name}: {e}");
                return werr!(500, "list versions: {e}");
            }
        };
        if vers.is_empty() {
            return werr!(404, "no versions for {name:?}");
        }
        let mut releases = Vec::with_capacity(vers.len());
        for ver in vers {
            let mut urls = std::collections::HashMap::new();
            let rdir = pdir.join(&ver);
            if let Ok(entries) = std::fs::read_dir(&rdir) {
                for f in entries.flatten() {
                    let fname = f.file_name().to_string_lossy().to_string();
                    if !f.path().is_dir() && fname != META_FILE {
                        urls.insert(fname.clone(), self.asset_url(req, name, &ver, &fname));
                    }
                }
            }
            let rel = match self.build_release(&pdir, &ver, &urls) {
                Ok(Some(r)) => r,
                Ok(None) => continue,
                Err(e) => {
                    eprintln!("updateserver: build release {name}/{ver}: {e}");
                    return werr!(500, "build release: {e}");
                }
            };
            releases.push(rel);
        }
        let body = serde_json::to_vec(&releases).unwrap_or_else(|_| b"[]".to_vec());
        Response::json(200, body)
    }

    /// GET /feeds.json — 每个产品与其版本列表。
    fn handle_feeds(&self) -> Response {
        let names = match self.list_products() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("updateserver: list products: {e}");
                return werr!(500, "list products: {e}");
            }
        };
        let mut feeds = Vec::new();
        for name in names {
            let vers = match self.list_versions(&name) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("updateserver: versions {name}: {e}");
                    continue;
                }
            };
            if vers.is_empty() {
                continue;
            }
            let mut f = serde_json::Map::new();
            f.insert("name".to_string(), serde_json::Value::String(name));
            f.insert(
                "latest_version".to_string(),
                serde_json::Value::String(vers[0].clone()),
            );
            f.insert(
                "versions".to_string(),
                serde_json::Value::Array(vers.into_iter().map(serde_json::Value::String).collect()),
            );
            feeds.push(serde_json::Value::Object(f));
        }
        let body = serde_json::to_vec(&serde_json::json!({ "feeds": feeds })).unwrap_or_default();
        Response::json(200, body)
    }

    /// GET /package/<name>/<version>/<file>。
    fn handle_download(&self, rest: &str) -> Response {
        let mut parts = rest.split('/');
        let name = parts.next().unwrap_or_default();
        let ver = parts.next().unwrap_or_default();
        let file = parts.collect::<Vec<_>>().join("/");
        if name.is_empty() || ver.is_empty() || file.is_empty() {
            return werr!(404, "not found");
        }
        let pdir = match self.product_dir(name) {
            Ok(p) => p,
            Err(_) => return werr!(400, "invalid product name"),
        };
        let full = pdir.join(ver).join(&file);
        let root = match std::fs::canonicalize(&pdir) {
            Ok(r) => r,
            Err(_) => return werr!(404, "not found"),
        };
        // 防御性校验：解析符号链接后必须落在 <dir>/package/<name> 之下
        let resolved = match std::fs::canonicalize(&full) {
            Ok(r) => r,
            Err(_) => return werr!(404, "not found"),
        };
        let within = resolved
            .strip_prefix(&root)
            .map(|rel| !rel.as_os_str().is_empty())
            .unwrap_or(false);
        if !within {
            return werr!(404, "not found");
        }
        let size = match std::fs::metadata(&resolved) {
            Ok(m) => m.len(),
            Err(_) => return werr!(404, "not found"),
        };
        match std::fs::File::open(&resolved) {
            Ok(f) => Response {
                status: 200,
                headers: vec![(
                    "Content-Type".to_string(),
                    content_type(&resolved).to_string(),
                )],
                body: httpd::Body::File(f, size),
            },
            Err(_) => werr!(404, "not found"),
        }
    }
}

/// 简单按扩展名推断 Content-Type。
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => "application/json",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// {"error": "..."} JSON 错误响应（与 Go writeError 一致）。
fn write_error(status: u16, msg: &str) -> Response {
    Response::json(
        status,
        format!(
            r#"{{"error":{}}}"#,
            serde_json::to_string(msg).unwrap_or_default()
        ),
    )
}

/// Unix 秒 → RFC3339（秒精度，UTC，恒为 Z；对应 Go "2006-01-02T15:04:05Z"）。
pub fn rfc3339_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let secs_of_day = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let h = secs_of_day / 3600;
    let mi = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// days（自 Unix 纪元）→ (年, 月, 日)；Howard Hinnant 民用历算法。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn must_mkdir(p: &Path) {
        fs::create_dir_all(p).unwrap();
    }
    fn must_write(p: &Path, content: &str) {
        fs::write(p, content).unwrap();
    }

    /// 与 Go server_test.go setup 同构的数据目录（每次调用唯一，可并行）。
    fn setup() -> (Server, String) {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "update-srv-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        must_mkdir(&dir.join("package/my-app/v2.0.0"));
        must_write(
            &dir.join("package/my-app/v2.0.0/app-linux-amd64.tar.gz"),
            "linux-content",
        );
        must_write(
            &dir.join("package/my-app/v2.0.0/app-windows-amd64.zip"),
            "win-content",
        );
        must_write(
            &dir.join("package/my-app/v2.0.0/meta.json"),
            r#"{
                "name": "My App",
                "notes": "second release",
                "published_at": "2024-02-01T00:00:00Z",
                "assets": {"app-linux-amd64.tar.gz": {"sha256": "deadbeef"}}
            }"#,
        );
        must_mkdir(&dir.join("package/my-app/v1.0.0"));
        must_write(
            &dir.join("package/my-app/v1.0.0/app-linux-amd64.tar.gz"),
            "old-linux",
        );
        must_mkdir(&dir.join("package/other-app/v0.1.0"));
        must_write(&dir.join("package/other-app/v0.1.0/app.bin"), "other");
        (
            Server::new(dir.to_str().unwrap()),
            dir.display().to_string(),
        )
    }

    fn req_with_host(path: &str, host: &str) -> Request {
        Request {
            method: "GET".to_string(),
            path: path.to_string(),
            query: String::new(),
            headers: vec![("Host".to_string(), host.to_string())],
            tls: false,
        }
    }

    fn get(ts: &Server, path: &str) -> (u16, String) {
        let resp = ts.handle(&req_with_host(path, "updates.example.com"));
        let body = match &resp.body {
            httpd::Body::Bytes(b) => String::from_utf8_lossy(b).to_string(),
            httpd::Body::None => String::new(),
            httpd::Body::File(_, _) => panic!("unexpected file body"),
        };
        (resp.status, body)
    }

    #[test]
    fn test_feed_sorted_newest_first() {
        let (ts, _dir) = setup();
        let (status, body) = get(&ts, "/feed/my-app.json");
        assert_eq!(status, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["version"], "v2.0.0");
        assert_eq!(arr[1]["version"], "v1.0.0");
        assert_eq!(arr[0]["notes"], "second release");
        assert_eq!(arr[0]["published_at"], "2024-02-01T00:00:00Z");
        // meta.json 排除、sha256 合并、url 指向下载端点
        let assets = arr[0]["assets"].as_array().unwrap();
        let linux = assets
            .iter()
            .find(|a| a["name"] == "app-linux-amd64.tar.gz")
            .unwrap();
        assert_eq!(linux["sha256"], "deadbeef");
        let url = linux["url"].as_str().unwrap();
        assert!(url.contains("/package/my-app/v2.0.0/"), "url = {url}");
        assert!(
            url.starts_with("http://updates.example.com/"),
            "url = {url}"
        );
        // 无 meta 的 v1.0.0：published_at 回退目录 mtime（非空）
        assert!(!arr[1]["published_at"].as_str().unwrap_or("").is_empty());
    }

    #[test]
    fn test_feed_semver_ordering() {
        let dir = std::env::temp_dir().join(format!("update-srv-ord-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for v in ["v1.9.0", "v1.10.0", "v2.0.0", "v1.2.0"] {
            must_mkdir(&dir.join("package/app").join(v));
        }
        let ts = Server::new(dir.to_str().unwrap());
        let (status, body) = get(&ts, "/feed/app.json");
        assert_eq!(status, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let got: Vec<&str> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["version"].as_str().unwrap())
            .collect();
        assert_eq!(got, vec!["v2.0.0", "v1.10.0", "v1.9.0", "v1.2.0"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_feed_unknown_product_404() {
        let (ts, _) = setup();
        let (status, _) = get(&ts, "/feed/nope.json");
        assert_eq!(status, 404);
    }

    #[test]
    fn test_feeds_list() {
        let (ts, _) = setup();
        let (status, body) = get(&ts, "/feeds.json");
        assert_eq!(status, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let feeds = v["feeds"].as_array().unwrap();
        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0]["name"], "my-app");
        assert_eq!(feeds[0]["latest_version"], "v2.0.0");
    }

    #[test]
    fn test_download_traversal_rejected() {
        let (ts, dir) = setup();
        fs::write(Path::new(&dir).join("secret.txt"), "top-secret").unwrap();
        for path in [
            "/package/../secret.txt",
            "/package/my-app/..%2F..%2Fsecret.txt",
            "/package/my-app/v2.0.0/../../secret.txt",
        ] {
            let (status, _) = get(&ts, path);
            assert_ne!(status, 200, "traversal {path} returned 200");
        }
        // 正常下载（File 流式响应）
        let resp = ts.handle(&req_with_host(
            "/package/my-app/v2.0.0/app-linux-amd64.tar.gz",
            "h",
        ));
        assert_eq!(resp.status, 200);
        match resp.body {
            httpd::Body::File(_, _) => {}
            _ => panic!("expected file body for normal download"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_invalid_product_name() {
        let dir = std::env::temp_dir().join(format!("update-srv-inv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ts = Server::new(dir.to_str().unwrap());
        let resp = ts.handle(&req_with_host("/feed/..%2F..%2Fetc.json", "h"));
        assert_eq!(resp.status, 400);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_product_dir_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("update-srv-pd-{}", std::process::id()));
        let ts = Server::new(dir.to_str().unwrap());
        for name in ["..", "../x", "a/b/../../x", "/abs", ""] {
            assert!(
                ts.product_dir(name).is_err(),
                "product_dir({name:?}) accepted"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_asset_url_scheme() {
        let dir = std::env::temp_dir().join(format!("update-srv-au-{}", std::process::id()));
        let ts = Server::new(dir.to_str().unwrap());
        let req = req_with_host("/package/a/b/f", "host");
        assert_eq!(
            ts.asset_url(&req, "a", "b", "f"),
            "http://host/package/a/b/f"
        );
        let req2 = Request {
            path: "/package/a/b/f".to_string(),
            query: String::new(),
            method: "GET".to_string(),
            headers: vec![
                ("Host".to_string(), "host".to_string()),
                ("X-Forwarded-Proto".to_string(), "https".to_string()),
            ],
            tls: false,
        };
        assert_eq!(
            ts.asset_url(&req2, "a", "b", "f"),
            "https://host/package/a/b/f"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_rfc3339_from_unix() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1706745600), "2024-02-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1706745663), "2024-02-01T00:01:03Z");
        assert_eq!(rfc3339_from_unix(-1), "1969-12-31T23:59:59Z");
        assert_eq!(rfc3339_from_unix(253402300799), "9999-12-31T23:59:59Z");
    }

    #[test]
    fn test_download_body_streams() {
        let (ts, dir) = setup();
        let resp = ts.handle(&req_with_host(
            "/package/my-app/v2.0.0/app-linux-amd64.tar.gz",
            "h",
        ));
        assert_eq!(resp.status, 200);
        match resp.body {
            httpd::Body::File(mut f, size) => {
                assert_eq!(size, "linux-content".len() as u64);
                use std::io::Read;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).unwrap();
                assert_eq!(buf, b"linux-content");
            }
            _ => panic!("expected file body"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_healthz() {
        let dir = std::env::temp_dir().join(format!("update-srv-hz-{}", std::process::id()));
        let ts = Server::new(dir.to_str().unwrap());
        let resp = ts.handle(&req_with_host("/healthz", "h"));
        assert_eq!(resp.status, 200);
        assert_eq!(
            match &resp.body {
                httpd::Body::Bytes(b) => String::from_utf8_lossy(b).to_string(),
                _ => String::new(),
            },
            r#"{"status":"ok"}"#
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_meta_checksum_and_asset_size_override() {
        // 对应 Go server_test.go 中 meta 对 size 的覆盖行为
        let dir = std::env::temp_dir().join(format!("update-srv-meta-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        must_mkdir(&dir.join("package/a/v1.0.0"));
        must_write(&dir.join("package/a/v1.0.0/app.bin"), "xyz");
        must_write(
            &dir.join("package/a/v1.0.0/meta.json"),
            r#"{"checksum": "abc", "assets": {"app.bin": {"size": 123, "sha256": "deadbeef"}}}"#,
        );
        let ts = Server::new(dir.to_str().unwrap());
        let (status, body) = get(&ts, "/feed/a.json");
        assert_eq!(status, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let rel = &v.as_array().unwrap()[0];
        assert_eq!(rel["checksum"], "abc");
        assert_eq!(rel["assets"][0]["size"], 123);
        assert_eq!(rel["assets"][0]["sha256"], "deadbeef");
        let _ = fs::remove_dir_all(&dir);
    }
}
