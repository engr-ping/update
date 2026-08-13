//! update 的 C ABI 共享库（libupdate.dll / libupdate.so / libupdate.dylib）。
//!
//! 宿主（C/C++/Python/…）通过本库的内存态接口调用 update-core 的 CLI 逻辑，
//! 无需拉起子进程。协议与 `update` 二进制一致（docs/integration.md §4）：
//! stdout 对应协议 JSON，stderr 对应错误/日志消息。
//!
//! 内存约定：本库所有返回字符串均由库内 malloc（Rust 分配器）分配，
//! 调用方必须用 `update_free` 释放。空串表示无内容（如 `update_last_error`
//! 返回 "" 表示无错误）。
//!
//! 失败约定：业务失败时返回值为错误消息字符串（永不返回 NULL），
//! 并同时设置 last_error；调用方流程：拿返回值 → 判断
//! `update_last_error` 是否为空 → 决定成败。

use std::ffi::{c_char, CStr, CString};
use std::sync::Mutex;

use update::{cli, versioninfo};

/// 最近一次失败的错误消息；成功调用时清空。
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

/// 读取 *const c_char 为 Option<String>；空指针/非法 UTF-8 → None。
fn cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p).to_str().ok().map(str::to_string) }
}

/// 把字符串包装为 *mut c_char（Box<CString>::into_raw 分配，update_free 回收）。
fn to_c(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => Box::into_raw(Box::new(c)).cast(),
        // 字符串内含 NUL 时退化为空串（正常业务输出不含 NUL）。
        Err(_) => Box::into_raw(Box::new(CString::new("").unwrap())).cast(),
    }
}

/// 记录失败消息并返回其 C 指针。
fn fail(msg: String) -> *mut c_char {
    let mut last = LAST_ERROR.lock().unwrap();
    *last = msg.clone();
    to_c(msg)
}

/// 记录成功（清空 last_error）并返回 stdout/结果字符串。
fn ok(s: String) -> *mut c_char {
    *LAST_ERROR.lock().unwrap() = String::new();
    to_c(s)
}

/// 记录失败消息并返回退出码（供返回 i32 的函数使用）。
fn fail_code(msg: String) -> i32 {
    *LAST_ERROR.lock().unwrap() = msg;
    update::cli::EXIT_USAGE
}

/// 检查是否有新版本。
///
/// \param config_path       配置文件路径（必填）
/// \param current_version   当前版本号，可空；非空时作为 --current-version 传入
/// \return 成功：check 的协议 JSON（stdout）；失败：错误消息（stderr）。
#[no_mangle]
pub extern "C" fn update_check(
    config_path: *const c_char,
    current_version: *const c_char,
) -> *mut c_char {
    let Some(cfg) = cstr(config_path) else {
        return fail("update_check: config_path is null or invalid UTF-8".into());
    };
    let mut args = vec![
        "update".to_string(),
        "check".to_string(),
        "--config".to_string(),
        cfg,
    ];
    if let Some(v) = cstr(current_version) {
        args.push("--current-version".to_string());
        args.push(v);
    }
    let (code, out, err) = cli::run_to_buffers(&args);
    if code == cli::EXIT_OK {
        ok(out)
    } else {
        fail(err)
    }
}

/// 下载指定版本产物。
///
/// \param config_path   配置文件路径（必填）
/// \param version       版本号或 "latest"（必填）
/// \param asset         资产名，可空；非空时作为 --asset 传入
/// \param out           输出路径，可空；非空时作为 --out 传入
/// \return 成功：download 的协议 JSON（stdout）；失败：错误消息（stderr）。
#[no_mangle]
pub extern "C" fn update_download(
    config_path: *const c_char,
    version: *const c_char,
    asset: *const c_char,
    out: *const c_char,
) -> *mut c_char {
    let Some(cfg) = cstr(config_path) else {
        return fail("update_download: config_path is null or invalid UTF-8".into());
    };
    let Some(ver) = cstr(version) else {
        return fail("update_download: version is null or invalid UTF-8".into());
    };
    let mut args = vec![
        "update".to_string(),
        "download".to_string(),
        "--config".to_string(),
        cfg,
        "--version".to_string(),
        ver,
    ];
    if let Some(a) = cstr(asset) {
        args.push("--asset".to_string());
        args.push(a);
    }
    if let Some(o) = cstr(out) {
        args.push("--out".to_string());
        args.push(o);
    }
    let (code, stdout, stderr) = cli::run_to_buffers(&args);
    if code == cli::EXIT_OK {
        ok(stdout)
    } else {
        fail(stderr)
    }
}

/// 列出可用版本。
///
/// \param config_path   配置文件路径（必填）
/// \param limit         最多列出几个版本；> 0 时作为 --limit 传入
/// \return 成功：list 的协议 JSON（stdout）；失败：错误消息（stderr）。
#[no_mangle]
pub extern "C" fn update_list(config_path: *const c_char, limit: i32) -> *mut c_char {
    let Some(cfg) = cstr(config_path) else {
        return fail("update_list: config_path is null or invalid UTF-8".into());
    };
    let mut args = vec![
        "update".to_string(),
        "list".to_string(),
        "--config".to_string(),
        cfg,
    ];
    if limit > 0 {
        args.push("--limit".to_string());
        args.push(limit.to_string());
    }
    let (code, out, err) = cli::run_to_buffers(&args);
    if code == cli::EXIT_OK {
        ok(out)
    } else {
        fail(err)
    }
}

/// 返回 update 库自身的版本号（纯文本，不经 CLI）。
#[no_mangle]
pub extern "C" fn update_version() -> *mut c_char {
    to_c(versioninfo::VERSION.to_string())
}

/// 后台自动更新（阻塞式循环，宿主应在自己起的线程里调用）。
///
/// 行为：
///   - 立即检查一次，有更新则下载到 out_dir；
///   - 之后每 interval_secs（默认 86400）检查一次；
///   - 若 watch_pid > 0，持续监测该进程，宿主退出且有就绪更新时执行 on_update；
///   - 若 once != 0，仅检查+下载一次即返回。
///
/// \param config_path   配置文件路径（必填）
/// \param interval_secs 检查间隔秒数（<=0 视为默认 1 天）
/// \param out_dir       下载目录（可空，默认系统临时目录）
/// \param watch_pid     宿主 PID（0 表示不监测）
/// \param on_update     宿主退出后执行的 shell 模板（可空，支持 {file}/{version}）
/// \return 退出码（0 成功）。
#[no_mangle]
pub extern "C" fn update_autoupdate_run(
    config_path: *const c_char,
    interval_secs: u64,
    out_dir: *const c_char,
    watch_pid: u32,
    on_update: *const c_char,
) -> i32 {
    let Some(cfg) = cstr(config_path) else {
        return fail_code("update_autoupdate_run: config_path is null or invalid UTF-8".into());
    };
    let opts = update::autoupdate::AutoOptions {
        config_path: cfg,
        interval_secs,
        out_dir: cstr(out_dir).unwrap_or_else(update::autoupdate::default_out_dir),
        watch_pid: if watch_pid == 0 {
            None
        } else {
            Some(watch_pid)
        },
        on_update: cstr(on_update),
        once: false,
        current_version: None,
        platform: None,
        username: None,
        password: None,
    };
    let mut stderr_buf: Vec<u8> = Vec::new();
    let code = update::autoupdate::run_autoupdate(&opts, &mut stderr_buf);
    if code == update::cli::EXIT_OK {
        *LAST_ERROR.lock().unwrap() = String::new();
    } else {
        *LAST_ERROR.lock().unwrap() = String::from_utf8_lossy(&stderr_buf).into_owned();
    }
    code
}

/// 宿主在退出钩子里显式应用已下载的更新。
///
/// \param template  shell 模板（支持 {file}/{version}）
/// \param file      已下载文件路径
/// \param version   版本号
/// \return 成功：空串；失败：错误消息。
#[no_mangle]
pub extern "C" fn update_apply(
    template: *const c_char,
    file: *const c_char,
    version: *const c_char,
) -> *mut c_char {
    let (Some(t), Some(f), Some(v)) = (cstr(template), cstr(file), cstr(version)) else {
        return fail("update_apply: template/file/version must be non-null UTF-8".into());
    };
    let mut stderr_buf: Vec<u8> = Vec::new();
    let applied = update::autoupdate::apply_update(&t, &f, &v, &mut stderr_buf);
    if applied {
        ok(String::new())
    } else {
        fail(String::from_utf8_lossy(&stderr_buf).into_owned())
    }
}

/// 返回最近一次失败的错误消息字符串；"" 表示无错误。成功调用时清空。
#[no_mangle]
pub extern "C" fn update_last_error() -> *mut c_char {
    to_c(LAST_ERROR.lock().unwrap().clone())
}

/// 释放本库所有返回函数分配的内存（空指针安全）。
#[no_mangle]
pub extern "C" fn update_free(p: *mut c_char) {
    if p.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(p.cast::<CString>()));
    }
}
