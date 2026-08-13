// CLI 入口逻辑（对应原 Go internal/cli）。
//
// 宿主 ⇄ CLI 契约（docs/integration.md §4）：
//   - stdout 只输出协议 JSON（version 子命令输出纯文本）
//   - 日志/错误一律走 stderr
//   - 退出码：0 成功 | 2 配置/用法错误 | 3 源错误 | 4 下载错误
use crate::config::{self, Config};
use crate::r#match::{clean_tag, host_platform, match_asset};
use crate::semver::compare;
use crate::source::{self, Release};
use crate::transport::{Auth, Client, Error, ErrorKind, Options};
use crate::versioninfo::VERSION;
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_SOURCE: i32 = 3;
pub const EXIT_DOWNLOAD: i32 = 4;

const SCHEMA_VERSION: i64 = 1;

pub fn run(args: &[String]) -> i32 {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    run_impl(args, &mut stdout, &mut stderr)
}

/// 运行并将 stdout/stderr 捕获为字符串（C ABI 复用）。
pub fn run_to_buffers(args: &[String]) -> (i32, String, String) {
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = run_impl(args, &mut out, &mut err);
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

/// 可测试的入口：输出写入 out，错误/日志写入 err。
fn run_impl(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let Some(cmd) = args.get(1) else {
        let _ = write!(err, "{}", usage_text());
        return EXIT_USAGE;
    };
    match cmd.as_str() {
        "check" => cmd_check(&args[2..], out, err),
        "autoupdate" => cmd_autoupdate(&args[2..], out, err),
        "download" => cmd_download(&args[2..], out, err),
        "list" => cmd_list(&args[2..], out, err),
        "version" => match writeln!(out, "{}", VERSION) {
            Ok(_) => EXIT_OK,
            Err(_) => EXIT_USAGE,
        },
        "help" | "--help" | "-h" => {
            let _ = write!(out, "{}", usage_text());
            EXIT_OK
        }
        other => {
            let _ = writeln!(err, "update: unknown command {other:?}");
            let _ = write!(err, "{}", usage_text());
            EXIT_USAGE
        }
    }
}

// ---------------------------------------------------------------------------
// 参数解析

struct Flags {
    value: HashMap<String, String>,
    flag: HashSet<String>,
}

impl Flags {
    fn get(&self, k: &str) -> Option<&str> {
        self.value.get(k).map(String::as_str)
    }
}

fn parse_flags(
    args: &[String],
    value_flags: &[&str],
    bool_flags: &[&str],
) -> Result<Flags, String> {
    let mut f = Flags {
        value: HashMap::new(),
        flag: HashSet::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if !a.starts_with("--") {
            return Err(format!("unexpected argument {a:?}"));
        }
        let body = &a[2..];
        if let Some((k, v)) = body.split_once('=') {
            if !value_flags.contains(&k) {
                return Err(format!("unknown flag --{k}"));
            }
            f.value.insert(k.to_string(), v.to_string());
        } else if value_flags.contains(&body) {
            i += 1;
            let v = args
                .get(i)
                .ok_or_else(|| format!("flag --{body} requires a value"))?;
            f.value.insert(body.to_string(), v.clone());
        } else if bool_flags.contains(&body) {
            f.flag.insert(body.to_string());
        } else {
            return Err(format!("unknown flag --{body}"));
        }
        i += 1;
    }
    Ok(f)
}

fn wants_help(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--help" || a == "-h" || a == "help")
}

// ---------------------------------------------------------------------------
// 公共小工具

/// 配置路径：--config > $UPDATE_CONFIG > 报错。
fn load_config(flags: &Flags) -> Result<Config, String> {
    let path = flags
        .get("config")
        .map(str::to_string)
        .or_else(|| std::env::var("UPDATE_CONFIG").ok())
        .ok_or_else(|| "config required: pass --config FILE or set UPDATE_CONFIG".to_string())?;
    config::load(&path, None)
}

/// 目标平台："all" → 不过滤；否则 "os/arch"；缺省取宿主平台。
fn platform(flags: &Flags) -> Result<(String, String), String> {
    match flags.get("platform") {
        Some("all") => Ok((String::new(), String::new())),
        Some(p) => match p.split_once('/') {
            Some((os, arch)) if !os.is_empty() && !arch.is_empty() => {
                Ok((os.to_string(), arch.to_string()))
            }
            _ => Err(format!(
                "invalid --platform {p:?} (want \"os/arch\", e.g. linux/amd64, or \"all\")"
            )),
        },
        None => Ok(host_platform()),
    }
}

/// 运行时凭据覆盖：传用户名 → Basic；仅传密码 → Bearer（docs/design.md §5）。
fn apply_auth(cfg: &mut Config, username: Option<&str>, password: Option<&str>) {
    let (ty, u, p) = match (username, password) {
        (Some(u), p) => ("basic", u, p.unwrap_or("")),
        (None, Some(t)) => ("bearer", "", t),
        (None, None) => return,
    };
    if let Some(g) = cfg.source.github.as_mut() {
        g.username = u.to_string();
        g.token = p.to_string();
    }
    if let Some(cu) = cfg.source.custom.as_mut() {
        let a = cu.auth.get_or_insert_with(Default::default);
        a.ty = ty.to_string();
        a.username = u.to_string();
        a.token = p.to_string();
    }
}

fn build_client(cfg: &Config) -> Client {
    let mut opts = Options::default();
    if let Some(g) = &cfg.source.github {
        if !g.username.is_empty() {
            opts.auth = Some(Auth {
                ty: "basic".to_string(),
                token: g.token.clone(),
                username: g.username.clone(),
            });
        } else if !g.token.is_empty() {
            opts.auth = Some(Auth {
                ty: "bearer".to_string(),
                token: g.token.clone(),
                username: String::new(),
            });
        }
    }
    if let Some(cu) = &cfg.source.custom {
        if let Some(a) = &cu.auth {
            if a.ty == "basic" {
                opts.auth = Some(Auth {
                    ty: "basic".to_string(),
                    token: a.token.clone(),
                    username: a.username.clone(),
                });
            } else if a.ty == "bearer" {
                opts.auth = Some(Auth {
                    ty: "bearer".to_string(),
                    token: a.token.clone(),
                    username: String::new(),
                });
            }
        }
        if let Some(h) = &cu.headers {
            opts.headers = h.clone();
        }
    }
    Client::new(opts)
}

/// product.asset_filter 正则（可选）。
fn compile_filter(cfg: &Config) -> Result<Option<Regex>, String> {
    if cfg.product.asset_filter.is_empty() {
        return Ok(None);
    }
    Regex::new(&cfg.product.asset_filter)
        .map(Some)
        .map_err(|e| {
            format!(
                "config: invalid asset_filter {:?}: {e}",
                cfg.product.asset_filter
            )
        })
}

/// 按平台 + asset_filter 过滤产物的副本筛除：缺省不过滤；"all" 已在上游转为空。
fn filter_assets(rel: &mut Release, os: &str, arch: &str, filter: Option<&Regex>) {
    rel.assets.retain(|a| {
        if !match_asset(&a.name, os, arch) {
            return false;
        }
        if let Some(re) = filter {
            if !re.is_match(&a.name) {
                return false;
            }
        }
        true
    });
}

// ---------------------------------------------------------------------------
// 子命令

#[derive(Serialize)]
struct CheckOutput {
    schema: i64,
    current_version: String,
    latest_version: String,
    update_available: bool,
    release: Release,
}

fn cmd_check(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if wants_help(args) {
        let _ = write!(out, "{}", check_usage());
        return EXIT_OK;
    }
    let flags = match parse_flags(
        args,
        &[
            "config",
            "current-version",
            "platform",
            "username",
            "password",
        ],
        &["json"],
    ) {
        Ok(f) => f,
        Err(e) => return usage_fail(&e, err),
    };
    let mut cfg = match load_config(&flags) {
        Ok(c) => c,
        Err(e) => return config_fail(&e, err),
    };
    let (os, arch) = match platform(&flags) {
        Ok(p) => p,
        Err(e) => return usage_fail(&e, err),
    };
    apply_auth(&mut cfg, flags.get("username"), flags.get("password"));
    let filter = match compile_filter(&cfg) {
        Ok(r) => r,
        Err(e) => return config_fail(&e, err),
    };
    let client = build_client(&cfg);
    let src = match source::new(&cfg, &client) {
        Ok(s) => s,
        Err(e) => return config_fail(&e, err),
    };
    let mut rel = match src.latest() {
        Ok(r) => r,
        Err(e) => return source_fail(&e, err),
    };
    filter_assets(&mut rel, &os, &arch, filter.as_ref());

    let current = flags
        .get("current-version")
        .map(str::to_string)
        .unwrap_or_else(|| cfg.product.current_version.clone());
    let latest = rel.version.clone();
    let update_available =
        !current.trim().is_empty() && compare(&clean_tag(&current), &clean_tag(&latest)) < 0;
    let obj = CheckOutput {
        schema: SCHEMA_VERSION,
        current_version: current,
        latest_version: latest,
        update_available,
        release: rel,
    };
    let _ = writeln!(out, "{}", serde_json::to_string(&obj).unwrap_or_default());
    EXIT_OK
}

fn cmd_autoupdate(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if wants_help(args) {
        let _ = write!(out, "{}", autoupdate_usage());
        return EXIT_OK;
    }
    let flags = match parse_flags(
        args,
        &[
            "config",
            "interval",
            "out",
            "watch-pid",
            "on-update",
            "current-version",
            "platform",
            "username",
            "password",
        ],
        &["once"],
    ) {
        Ok(f) => f,
        Err(e) => return usage_fail(&e, err),
    };
    let config_path = match flags.get("config") {
        Some(c) => c.to_string(),
        None => {
            return config_fail(
                "autoupdate requires --config FILE (or set UPDATE_CONFIG)",
                err,
            )
        }
    };
    let interval_secs = flags
        .get("interval")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(86400);
    let out_dir = flags
        .get("out")
        .map(str::to_string)
        .unwrap_or_else(crate::autoupdate::default_out_dir);
    let watch_pid = flags.get("watch-pid").and_then(|s| s.parse::<u32>().ok());
    let on_update = flags.get("on-update").map(str::to_string);
    let once = flags.flag.contains("once");
    let current_version = flags.get("current-version").map(str::to_string);
    let platform = flags.get("platform").map(str::to_string);
    let username = flags.get("username").map(str::to_string);
    let password = flags.get("password").map(str::to_string);

    let opts = crate::autoupdate::AutoOptions {
        config_path,
        interval_secs,
        out_dir,
        watch_pid,
        on_update,
        once,
        current_version,
        platform,
        username,
        password,
    };
    crate::autoupdate::run_autoupdate(&opts, err)
}

#[derive(Serialize)]
struct DownloadOutput {
    schema: i64,
    version: String,
    file: String,
}

fn cmd_download(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if wants_help(args) {
        let _ = write!(out, "{}", download_usage());
        return EXIT_OK;
    }
    let flags = match parse_flags(
        args,
        &[
            "config", "version", "asset", "out", "platform", "username", "password",
        ],
        &[],
    ) {
        Ok(f) => f,
        Err(e) => return usage_fail(&e, err),
    };
    let mut cfg = match load_config(&flags) {
        Ok(c) => c,
        Err(e) => return config_fail(&e, err),
    };
    let (os, arch) = match platform(&flags) {
        Ok(p) => p,
        Err(e) => return usage_fail(&e, err),
    };
    apply_auth(&mut cfg, flags.get("username"), flags.get("password"));
    let filter = match compile_filter(&cfg) {
        Ok(r) => r,
        Err(e) => return config_fail(&e, err),
    };
    let client = build_client(&cfg);
    let src = match source::new(&cfg, &client) {
        Ok(s) => s,
        Err(e) => return config_fail(&e, err),
    };

    let want = match flags.get("version") {
        Some(v) => v,
        None => return usage_fail("download requires --version (a version or \"latest\")", err),
    };
    let mut rel = if want == "latest" {
        match src.latest() {
            Ok(r) => r,
            Err(e) => return source_fail(&e, err),
        }
    } else {
        let want_clean = clean_tag(want);
        match src.list(1000) {
            Ok(rels) => {
                match rels
                    .into_iter()
                    .find(|r| clean_tag(&r.version) == want_clean)
                {
                    Some(r) => r,
                    None => {
                        let _ = writeln!(err, "update: release {want:?} not found");
                        return EXIT_SOURCE;
                    }
                }
            }
            Err(e) => return source_fail(&e, err),
        }
    };
    filter_assets(&mut rel, &os, &arch, filter.as_ref());

    let asset = match flags.get("asset") {
        Some(spec) => rel
            .assets
            .iter()
            .find(|a| a.name == spec || a.name.contains(spec)),
        None => rel.assets.first(),
    };
    let asset = match asset {
        Some(a) => a,
        None => {
            let p = if os.is_empty() {
                "all".to_string()
            } else {
                format!("{os}/{arch}")
            };
            let _ = writeln!(err, "update: no matching asset for platform {p}");
            return EXIT_SOURCE;
        }
    };
    if asset.url.is_empty() {
        let _ = writeln!(err, "update: asset {:?} has no download url", asset.name);
        return EXIT_SOURCE;
    }

    // 目标路径：--out 为目录（或以 / 结尾）时拼接资产名，否则作为文件路径。
    let out_flag = flags.get("out").unwrap_or(".").to_string();
    let dest = if out_flag.ends_with('/') || Path::new(&out_flag).is_dir() {
        Path::new(&out_flag).join(&asset.name)
    } else {
        Path::new(&out_flag).to_path_buf()
    };
    let dest_str = dest.to_string_lossy().to_string();

    match client.download(&asset.url, &dest_str, &asset.sha256) {
        Ok(_) => {}
        Err(e) => {
            let _ = writeln!(err, "update: {}", e.message);
            return match e.kind {
                ErrorKind::Download => EXIT_DOWNLOAD,
                ErrorKind::Source => EXIT_SOURCE,
            };
        }
    }
    let obj = DownloadOutput {
        schema: SCHEMA_VERSION,
        version: rel.version,
        file: dest_str,
    };
    let _ = writeln!(out, "{}", serde_json::to_string(&obj).unwrap_or_default());
    EXIT_OK
}

#[derive(Serialize)]
struct ListOutput {
    schema: i64,
    versions: Vec<Release>,
}

fn cmd_list(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if wants_help(args) {
        let _ = write!(out, "{}", list_usage());
        return EXIT_OK;
    }
    let flags = match parse_flags(
        args,
        &["config", "limit", "platform", "username", "password"],
        &["json"],
    ) {
        Ok(f) => f,
        Err(e) => return usage_fail(&e, err),
    };
    let mut cfg = match load_config(&flags) {
        Ok(c) => c,
        Err(e) => return config_fail(&e, err),
    };
    let (os, arch) = match platform(&flags) {
        Ok(p) => p,
        Err(e) => return usage_fail(&e, err),
    };
    apply_auth(&mut cfg, flags.get("username"), flags.get("password"));
    let filter = match compile_filter(&cfg) {
        Ok(r) => r,
        Err(e) => return config_fail(&e, err),
    };
    let limit = match flags.get("limit") {
        Some(v) => match v.parse::<usize>() {
            Ok(n) if n >= 1 => n,
            _ => {
                return usage_fail(
                    &format!("invalid --limit {v:?} (want an integer >= 1)"),
                    err,
                )
            }
        },
        None => 10,
    };
    let client = build_client(&cfg);
    let src = match source::new(&cfg, &client) {
        Ok(s) => s,
        Err(e) => return config_fail(&e, err),
    };
    let mut rels = match src.list(limit) {
        Ok(r) => r,
        Err(e) => return source_fail(&e, err),
    };
    for r in &mut rels {
        filter_assets(r, &os, &arch, filter.as_ref());
    }
    let obj = ListOutput {
        schema: SCHEMA_VERSION,
        versions: rels,
    };
    let _ = writeln!(out, "{}", serde_json::to_string(&obj).unwrap_or_default());
    EXIT_OK
}

// ---------------------------------------------------------------------------
// 错误与帮助文本

fn usage_fail(msg: &str, err: &mut dyn Write) -> i32 {
    let _ = writeln!(err, "update: {msg}");
    EXIT_USAGE
}

fn config_fail(msg: &str, err: &mut dyn Write) -> i32 {
    let _ = writeln!(err, "update: {msg}");
    EXIT_USAGE
}

fn source_fail(e: &Error, err: &mut dyn Write) -> i32 {
    let _ = writeln!(err, "update: {}", e.message);
    match e.kind {
        ErrorKind::Source => EXIT_SOURCE,
        ErrorKind::Download => EXIT_DOWNLOAD,
    }
}

fn usage_text() -> String {
    format!(
        "update {VERSION} — 通用软件更新工具（Rust 版）

用法: update <命令> [选项]

命令:
  check      检查是否有新版本
  download   下载指定版本产物
  list       列出可用版本
  version    输出版本号（纯文本）
  help       显示本帮助

退出码: 0 成功 | 2 配置/用法错误 | 3 源错误 | 4 下载错误

运行 \"update <命令> --help\" 查看命令详情。
"
    )
}

fn check_usage() -> &'static str {
    "update check — 检查是否有新版本

用法: update check [选项]
  --config FILE          配置文件（缺省取 $UPDATE_CONFIG）
  --current-version X    当前版本号，覆盖配置里的 current_version
  --platform os/arch     目标平台（默认宿主；\"all\" 不过滤资产）
  --username USER        认证用户名（触发 Basic 认证）
  --password PASS        密码（无 --username 时视为 Bearer token）
  --json                 显式要求 JSON 输出（默认即 JSON，为兼容保留）

stdout 输出协议 JSON：schema / current_version / latest_version /
update_available / release（assets 已按平台过滤）。
"
}

fn download_usage() -> &'static str {
    "update download — 下载指定版本产物

用法: update download --version X [选项]
  --config FILE          配置文件（缺省取 $UPDATE_CONFIG）
  --version X            版本号或 \"latest\"（必填）
  --asset NAME           只下载名字匹配该值的资产（精确或包含）
  --out PATH             输出路径；目录（或以 / 结尾）时拼接资产名，默认当前目录
  --platform os/arch     目标平台（默认宿主；\"all\" 不过滤资产）
  --username USER        认证用户名（触发 Basic 认证）
  --password PASS        密码（无 --username 时视为 Bearer token）

stdout 输出协议 JSON：schema / version / file。
"
}

fn autoupdate_usage() -> &'static str {
    "update autoupdate — 后台自动检查并下载更新

用法: update autoupdate [选项]
  --config FILE          配置文件（必填，或设置 $UPDATE_CONFIG）
  --interval N           两次检查的间隔秒数，默认 86400（1 天）
  --out DIR              下载目录，默认系统临时目录（按进程隔离）
  --watch-pid PID        监测宿主进程；宿主退出且有就绪更新时执行 --on-update
  --on-update CMD        宿主退出后执行的 shell 命令，支持 {file} {version} 占位符
  --once                 仅检查并下载一次即退出（不进入循环）
  --current-version X    覆盖配置里的当前版本
  --platform os/arch     目标平台（默认宿主；\"all\" 不过滤资产）
  --username USER        认证用户名（触发 Basic 认证）
  --password PASS        密码（无 --username 时视为 Bearer token）

典型用法（独立进程，不影响宿主）：
  update autoupdate --config x.json --watch-pid $PPID --on-update 'mv {file} /opt/app && /opt/app'
"
}

fn list_usage() -> &'static str {
    "update list — 列出可用版本
  --config FILE          配置文件（缺省取 $UPDATE_CONFIG）
  --limit N              最多列出几个版本，默认 10
  --platform os/arch     目标平台（默认宿主；\"all\" 不过滤资产）
  --username USER        认证用户名（触发 Basic 认证）
  --password PASS        密码（无 --username 时视为 Bearer token）
  --json                 显式要求 JSON 输出（默认即 JSON，为兼容保留）

stdout 输出协议 JSON：schema / versions（最新在前）。
"
}

// ---------------------------------------------------------------------------
// 测试

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{json_response, text_response, TestServer};

    fn run_args(args: &[String]) -> (i32, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_impl(args, &mut out, &mut err);
        (
            code,
            String::from_utf8_lossy(&out).to_string(),
            String::from_utf8_lossy(&err).to_string(),
        )
    }

    fn write_temp(dir: &str, name: &str, content: &str) -> String {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let p = std::env::temp_dir()
            .join(format!(
                "update-cli-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ))
            .join(dir);
        std::fs::create_dir_all(&p).unwrap();
        let f = p.join(name);
        std::fs::write(&f, content).unwrap();
        f.to_string_lossy().to_string()
    }

    /// 测试用的输出目录（不存在则创建）。
    fn out_dir(name: &str) -> String {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let p = std::env::temp_dir()
            .join(format!(
                "update-cli-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ))
            .join(name);
        std::fs::create_dir_all(&p).unwrap();
        p.to_string_lossy().to_string()
    }

    /// 两个版本、两个平台的 feed；/dl/<name> 返回 "hello <name>" 内容。
    fn feed_server() -> (TestServer, String) {
        let srv = TestServer::new(move |req| {
            if let Some(name) = req.path.strip_prefix("/dl/") {
                return text_response(200, &format!("hello {name}"));
            }
            if req.path == "/feed.json" {
                return json_response(
                    200,
                    &format!(
                        r#"[
                          {{"version":"2.0.0","published_at":"2024-01-15T00:00:00Z",
                            "notes":"bug fixes",
                            "assets":[
                              {{"name":"app-linux-amd64.tar.gz","url":"http://{0}/dl/app-linux-amd64.tar.gz","size":10}},
                              {{"name":"app-windows-amd64.zip","url":"http://{0}/dl/app-windows-amd64.zip","size":20}}
                            ]}},
                          {{"version":"1.0.0","published_at":"2024-01-01T00:00:00Z",
                            "assets":[{{"name":"app-linux-amd64.tar.gz","url":"http://{0}/dl/app-linux-amd64.tar.gz","size":5}}]}}
                        ]"#,
                        req.host()
                    ),
                );
            }
            crate::testutil::text_response(404, "not found")
        });
        let cfg = write_temp(
            "cfg1",
            "config.json",
            &format!(
                r#"{{"product":{{"name":"my-app","current_version":"1.0.0"}},"source":{{"type":"custom","custom":{{"versions_url":"{}/feed.json"}}}}}}"#,
                srv.url
            ),
        );
        (srv, cfg)
    }

    #[test]
    fn test_version_command_plain_text() {
        let (code, out, _) = run_args(&["update".to_string(), "version".to_string()]);
        assert_eq!(code, EXIT_OK);
        assert_eq!(out.trim(), VERSION);
    }

    #[test]
    fn test_help_commands() {
        for cmd in ["help", "--help", "-h"] {
            let (code, out, _) = run_args(&["update".to_string(), cmd.to_string()]);
            assert_eq!(code, EXIT_OK, "{cmd}");
            assert!(out.contains("用法: update"), "{cmd}: {out}");
        }
        let (code, out, _) = run_args(&[
            "update".to_string(),
            "check".to_string(),
            "--help".to_string(),
        ]);
        assert_eq!(code, EXIT_OK);
        assert!(out.contains("update check"), "{out}");
    }

    #[test]
    fn test_unknown_command_and_flag() {
        let (code, _, err) = run_args(&["update".to_string(), "bogus".to_string()]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("unknown command"), "{err}");
        let (code, _, _) = run_args(&[
            "update".to_string(),
            "check".to_string(),
            "--nope".to_string(),
        ]);
        assert_eq!(code, EXIT_USAGE);
        let (code, _, _) = run_args(&["update".to_string(), "check".to_string()]);
        assert_eq!(code, EXIT_USAGE); // 无 --config 且无 UPDATE_CONFIG
    }

    #[test]
    fn test_check_update_available() {
        let (_srv, cfg) = feed_server();
        let args = [
            "update",
            "check",
            "--config",
            &cfg,
            "--platform",
            "linux/amd64",
        ]
        .map(str::to_string);
        let (code, out, err) = run_args(&args);
        assert_eq!(code, EXIT_OK, "{err}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["current_version"], "1.0.0");
        assert_eq!(v["latest_version"], "2.0.0");
        assert_eq!(v["update_available"], true);
        assert_eq!(v["release"]["version"], "2.0.0");
        assert_eq!(v["release"]["notes"], "bug fixes");
        let assets = v["release"]["assets"].as_array().unwrap();
        assert_eq!(assets.len(), 1, "应只剩 linux 资产: {assets:?}");
        assert_eq!(assets[0]["name"], "app-linux-amd64.tar.gz");
    }

    #[test]
    fn test_check_no_update_and_current_version_flag() {
        let (_srv, cfg) = feed_server();
        let args = [
            "update",
            "check",
            "--config",
            &cfg,
            "--current-version",
            "3.0.0",
            "--platform",
            "all",
        ]
        .map(str::to_string);
        let (code, out, _) = run_args(&args);
        assert_eq!(code, EXIT_OK);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["update_available"], false);
        let assets = v["release"]["assets"].as_array().unwrap();
        assert_eq!(assets.len(), 2, "--platform all 不过滤");
    }

    #[test]
    fn test_check_source_error_exit_3() {
        let srv = TestServer::new(|_| crate::testutil::text_response(500, "boom"));
        let cfg = write_temp(
            "cfg_err",
            "config.json",
            &format!(
                r#"{{"source":{{"type":"custom","custom":{{"versions_url":"{}/feed.json"}}}}}}"#,
                srv.url
            ),
        );
        let args = ["update", "check", "--config", &cfg].map(str::to_string);
        let (code, _, err) = run_args(&args);
        assert_eq!(code, EXIT_SOURCE);
        assert!(err.contains("500"), "{err}");
    }

    #[test]
    fn test_download_latest_to_dir() {
        let (_srv, cfg) = feed_server();
        let out_dir = out_dir("dl1");
        let args = [
            "update",
            "download",
            "--config",
            &cfg,
            "--version",
            "latest",
            "--platform",
            "linux/amd64",
            "--out",
            &out_dir,
        ]
        .map(str::to_string);
        let (code, out, err) = run_args(&args);
        assert_eq!(code, EXIT_OK, "{err}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["version"], "2.0.0");
        let file = v["file"].as_str().unwrap();
        assert!(file.ends_with("app-linux-amd64.tar.gz"), "{file}");
        assert_eq!(
            std::fs::read_to_string(file).unwrap(),
            "hello app-linux-amd64.tar.gz"
        );
    }

    #[test]
    fn test_download_specific_version_and_asset() {
        let (_srv, cfg) = feed_server();
        let out_dir = out_dir("dl2");
        let args = [
            "update",
            "download",
            "--config",
            &cfg,
            "--version",
            "2.0.0",
            "--platform",
            "all",
            "--asset",
            "windows",
            "--out",
            &out_dir,
        ]
        .map(str::to_string);
        let (code, out, _) = run_args(&args);
        assert_eq!(code, EXIT_OK);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["file"]
            .as_str()
            .unwrap()
            .ends_with("app-windows-amd64.zip"));
    }

    #[test]
    fn test_download_checksum_mismatch_exit_4() {
        let srv = TestServer::new(|req| {
            if req.path == "/feed.json" {
                json_response(
                    200,
                    &format!(
                        r#"[{{"version":"1.0.0","assets":[{{"name":"app.bin","url":"http://{}/dl/app.bin","size":5,"sha256":"deadbeef"}}]}}]"#,
                        req.host()
                    ),
                )
            } else {
                crate::testutil::text_response(200, "hello")
            }
        });
        let cfg = write_temp(
            "cfg4",
            "config.json",
            &format!(
                r#"{{"source":{{"type":"custom","custom":{{"versions_url":"{}/feed.json"}}}}}}"#,
                srv.url
            ),
        );
        let out_dir = out_dir("dl4");
        let args = [
            "update",
            "download",
            "--config",
            &cfg,
            "--version",
            "latest",
            "--platform",
            "linux/amd64",
            "--out",
            &out_dir,
        ]
        .map(str::to_string);
        let (code, _, err) = run_args(&args);
        assert_eq!(code, EXIT_DOWNLOAD);
        assert!(err.contains("sha256"), "{err}");
    }

    #[test]
    fn test_list_limit_and_order() {
        let (_srv, cfg) = feed_server();
        let args = ["update", "list", "--config", &cfg, "--limit", "1"].map(str::to_string);
        let (code, out, _) = run_args(&args);
        assert_eq!(code, EXIT_OK);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let versions = v["versions"].as_array().unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0]["version"], "2.0.0");
    }

    #[test]
    fn test_download_missing_version_arg() {
        let (_srv, cfg) = feed_server();
        let args = ["update", "download", "--config", &cfg].map(str::to_string);
        let (code, _, err) = run_args(&args);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("--version"), "{err}");
    }

    #[test]
    fn test_download_version_not_found() {
        let (_srv, cfg) = feed_server();
        let args = [
            "update",
            "download",
            "--config",
            &cfg,
            "--version",
            "9.9.9",
            "--platform",
            "linux/amd64",
        ]
        .map(str::to_string);
        let (code, _, err) = run_args(&args);
        assert_eq!(code, EXIT_SOURCE);
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn test_asset_filter_regex_from_config() {
        let srv = TestServer::new(|req| {
            json_response(
                200,
                &format!(
                    r#"[{{"version":"1.0.0","assets":[
                        {{"name":"app-full.bin","url":"{}/dl/app-full.bin","size":1}},
                        {{"name":"app-lite.bin","url":"{}/dl/app-lite.bin","size":1}}
                    ]}}]"#,
                    req.host(),
                    req.host()
                ),
            )
        });
        let cfg = write_temp(
            "cfg_re",
            "config.json",
            &format!(
                r#"{{"product":{{"name":"my-app","asset_filter":"^app-(full|pro)"}},"source":{{"type":"custom","custom":{{"versions_url":"{}/feed.json"}}}}}}"#,
                srv.url
            ),
        );
        let args = ["update", "check", "--config", &cfg, "--platform", "all"].map(str::to_string);
        let (code, out, _) = run_args(&args);
        assert_eq!(code, EXIT_OK);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let assets = v["release"]["assets"].as_array().unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0]["name"], "app-full.bin");
    }

    #[test]
    fn test_autoupdate_once_downloads() {
        let (_srv, cfg) = feed_server();
        let dl = out_dir("autodl_once");
        let args = [
            "update",
            "autoupdate",
            "--config",
            &cfg,
            "--once",
            "--out",
            &dl,
        ]
        .map(str::to_string);
        let (code, _, err) = run_args(&args);
        assert_eq!(code, EXIT_OK, "err: {err}");
        let f = std::path::Path::new(&dl).join("app-linux-amd64.tar.gz");
        assert!(f.exists(), "expected downloaded file in {dl}");
        let content = std::fs::read_to_string(&f).unwrap();
        assert_eq!(content, "hello app-linux-amd64.tar.gz");
    }

    #[test]
    fn test_autoupdate_once_no_update_skips_download() {
        // current_version 已是 2.0.0，无更新 → 不应下载任何东西。
        let srv = TestServer::new(move |req| {
            if let Some(name) = req.path.strip_prefix("/dl/") {
                return text_response(200, &format!("hello {name}"));
            }
            if req.path == "/feed.json" {
                return json_response(
                    200,
                    &format!(
                        r#"[{{"version":"2.0.0","assets":[{{"name":"app-linux-amd64.tar.gz","url":"http://{0}/dl/app-linux-amd64.tar.gz","size":10}}]}}]"#,
                        req.host()
                    ),
                );
            }
            crate::testutil::text_response(404, "not found")
        });
        let cfg = write_temp(
            "cfg_no-up",
            "config.json",
            &format!(
                r#"{{"product":{{"name":"my-app","current_version":"2.0.0"}},"source":{{"type":"custom","custom":{{"versions_url":"{}/feed.json"}}}}}}"#,
                srv.url
            ),
        );
        let dl = out_dir("autodl_noup");
        let args = [
            "update",
            "autoupdate",
            "--config",
            &cfg,
            "--once",
            "--out",
            &dl,
        ]
        .map(str::to_string);
        let (code, _, err) = run_args(&args);
        assert_eq!(code, EXIT_OK, "err: {err}");
        let entries = std::fs::read_dir(&dl).unwrap().count();
        assert_eq!(entries, 0, "should not download when no update");
    }

    #[test]
    fn test_autoupdate_watch_pid_applies_on_exit() {
        // 用一个真实子进程作为「宿主」：autoupdate 在独立线程里 watch 它，
        // 宿主被 kill 并回收后，autoupdate 感知其退出并执行 on-update（写标记文件）。
        let (_srv, cfg) = feed_server();
        let dl = out_dir("autodl_watch");
        let marker = dl.trim_end_matches('/').to_string() + "/applied.txt";
        let mut host = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn host");
        let pid = host.id();
        let on_update = format!("echo applied 2.0.0 > '{}'", marker);
        let args = [
            "update",
            "autoupdate",
            "--config",
            &cfg,
            "--watch-pid",
            &pid.to_string(),
            "--on-update",
            &on_update,
            "--out",
            &dl,
        ]
        .map(str::to_string);
        let handle = std::thread::spawn(move || run_args(&args));
        // 让 autoupdate 先完成首次检查+下载，再杀掉宿主。
        std::thread::sleep(std::time::Duration::from_millis(400));
        let _ = host.kill();
        let _ = host.wait();
        let (code, _, err) = handle.join().expect("autoupdate thread");
        assert_eq!(code, EXIT_OK, "err: {err}");
        // 宿主已退出，on-update 应已执行。
        assert!(
            std::path::Path::new(&marker).exists(),
            "on-update should run after host exits"
        );
        let content = std::fs::read_to_string(&marker).unwrap();
        assert!(content.contains("applied 2.0.0"), "got: {content}");
    }
}
