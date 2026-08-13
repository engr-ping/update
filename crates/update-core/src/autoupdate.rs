// 后台自动更新（对应「宿主 app 调用后自动检查/下载，关闭后应用」需求）。
//
// 设计目标（见 README「宿主集成手册」）：
//   - 独立运行：可作为独立进程（update autoupdate --watch-pid <宿主PID>）或被宿主
//     在自己起的线程里调用 update_autoupdate_run，完全不阻塞宿主主逻辑。
//   - 挂起：无任务时线程 sleep，CPU 占用为 0；仅到点（默认 1 天）或宿主退出时唤醒。
//   - 自动检查 + 自动下载：到点后复用 check/download 逻辑。
//   - 关闭后自动更新：--watch-pid 监测宿主进程，宿主退出且有就绪更新时执行
//     --on-update 钩子（解压/替换由宿主脚本完成，库保持平台无关安全）。
use crate::cli;
use std::io::Write;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

/// 自动更新选项（由 CLI 或 C ABI 填充）。
pub struct AutoOptions {
    pub config_path: String,
    /// 两次检查的间隔（秒），<=0 视为默认 1 天。
    pub interval_secs: u64,
    /// 下载目录（自动创建）。
    pub out_dir: String,
    /// 宿主进程 PID；给定后监测其存活，宿主退出时若有就绪更新则执行 on_update。
    pub watch_pid: Option<u32>,
    /// 宿主退出后执行的 shell 模板，支持 {file} {version} 占位符。
    pub on_update: Option<String>,
    /// 仅检查并下载一次即退出（不进入循环）。
    pub once: bool,
    /// 覆盖配置中的当前版本。
    pub current_version: Option<String>,
    /// 覆盖目标平台（"os/arch" 或 "all"）。
    pub platform: Option<String>,
    /// 运行时 Basic 用户名（透传给 check/download）。
    pub username: Option<String>,
    /// 运行时凭据（Bearer token 或 Basic 密码）。
    pub password: Option<String>,
}

const DEFAULT_INTERVAL_SECS: u64 = 86400;
/// 心跳/挂起粒度：每 2 秒醒来一次检查宿主存活（轻量）。
const TICK: Duration = Duration::from_secs(2);

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .output()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.contains(&format!("PID: {}", pid)) || s.contains(&format!(" {}", pid))
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn run_shell(cmd: &str) -> std::io::Result<std::process::ExitStatus> {
    Command::new("cmd").args(["/C", cmd]).status()
}

#[cfg(not(windows))]
fn run_shell(cmd: &str) -> std::io::Result<std::process::ExitStatus> {
    Command::new("sh").args(["-c", cmd]).status()
}

/// 执行 on_update 模板（替换 {file} {version}），返回是否成功。
fn run_on_update(template: &str, file: &str, version: &str, err: &mut dyn Write) -> bool {
    let cmd = template
        .replace("{file}", file)
        .replace("{version}", version);
    match run_shell(&cmd) {
        Ok(s) if s.success() => true,
        Ok(s) => {
            let _ = writeln!(err, "update: on-update command failed: {s}");
            false
        }
        Err(e) => {
            let _ = writeln!(err, "update: on-update command error: {e}");
            false
        }
    }
}

/// 检查是否有更新；有则返回最新版本号。
fn do_check(opts: &AutoOptions) -> Option<String> {
    let mut args = vec![
        "update".to_string(),
        "check".to_string(),
        "--config".to_string(),
        opts.config_path.clone(),
    ];
    if let Some(v) = &opts.current_version {
        args.push("--current-version".to_string());
        args.push(v.clone());
    }
    if let Some(p) = &opts.platform {
        args.push("--platform".to_string());
        args.push(p.clone());
    }
    if let Some(u) = &opts.username {
        args.push("--username".to_string());
        args.push(u.clone());
    }
    if let Some(p) = &opts.password {
        args.push("--password".to_string());
        args.push(p.clone());
    }
    let (code, out, _) = cli::run_to_buffers(&args);
    if code != cli::EXIT_OK {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&out).ok()?;
    let upd = v.get("update_available")?.as_bool()?;
    if !upd {
        return None;
    }
    v.get("latest_version")?.as_str().map(str::to_string)
}

/// 下载指定版本到 out_dir，返回实际文件路径。
fn do_download(opts: &AutoOptions, version: &str) -> Option<String> {
    let mut args = vec![
        "update".to_string(),
        "download".to_string(),
        "--config".to_string(),
        opts.config_path.clone(),
        "--version".to_string(),
        version.to_string(),
        "--out".to_string(),
        opts.out_dir.clone(),
    ];
    if let Some(p) = &opts.platform {
        args.push("--platform".to_string());
        args.push(p.clone());
    }
    if let Some(u) = &opts.username {
        args.push("--username".to_string());
        args.push(u.clone());
    }
    if let Some(p) = &opts.password {
        args.push("--password".to_string());
        args.push(p.clone());
    }
    let (code, out, _) = cli::run_to_buffers(&args);
    if code != cli::EXIT_OK {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&out).ok()?;
    v.get("file")?.as_str().map(str::to_string)
}

/// 运行自动更新循环。返回退出码。
///
/// - 立即检查一次；有更新则下载到 out_dir。
/// - 之后每 interval（默认 1 天）检查一次。
/// - --watch-pid 给定时持续监测宿主存活；宿主退出且有就绪更新则执行 on_update 后退出。
/// - --once 仅检查+下载一次即退出。
pub fn run_autoupdate(opts: &AutoOptions, err: &mut dyn Write) -> i32 {
    let interval = if opts.interval_secs == 0 {
        DEFAULT_INTERVAL_SECS
    } else {
        opts.interval_secs
    };
    let interval = Duration::from_secs(interval);

    if let Err(e) = std::fs::create_dir_all(&opts.out_dir) {
        let _ = writeln!(err, "update: create out dir {}: {e}", opts.out_dir);
        return cli::EXIT_USAGE;
    }

    // 立即先检查一次（last_check 设为 interval 之前）。
    let mut last_check = Instant::now() - interval;
    // (下载文件路径, 版本)
    let mut pending: Option<(String, String)> = None;

    loop {
        // 监测宿主存活（watch 模式）。
        if let Some(pid) = opts.watch_pid {
            if !process_alive(pid) {
                if let Some((file, ver)) = pending.clone() {
                    if let Some(tpl) = &opts.on_update {
                        let _ = writeln!(
                            err,
                            "update: host process {} exited; applying update {}",
                            pid, ver
                        );
                        run_on_update(tpl, &file, &ver, err);
                    }
                }
                return cli::EXIT_OK;
            }
        }

        let now = Instant::now();
        if now.duration_since(last_check) >= interval {
            last_check = now;
            if let Some(latest) = do_check(opts) {
                // 已下载过该版本则跳过，避免每天重复下载。
                let need = pending.as_ref().map(|(_, v)| v != &latest).unwrap_or(true);
                if need {
                    if let Some(file) = do_download(opts, &latest) {
                        let _ = writeln!(err, "update: downloaded {} -> {}", latest, file);
                        pending = Some((file, latest));
                    }
                }
            }
            if opts.once {
                return cli::EXIT_OK;
            }
        }

        if opts.once {
            return cli::EXIT_OK;
        }
        thread::sleep(TICK);
    }
}

/// 供宿主在退出钩子中显式执行更新（线程模式下宿主进程不会随 autoupdate 退出，
/// 需自行调用以应用已下载的更新）。
pub fn apply_update(template: &str, file: &str, version: &str, err: &mut dyn Write) -> bool {
    run_on_update(template, file, version, err)
}

/// 默认下载目录（按进程隔离，避免多实例冲突）。
pub fn default_out_dir() -> String {
    std::env::temp_dir()
        .join(format!("update-autodl-{}", std::process::id()))
        .to_string_lossy()
        .to_string()
}
