// 构建期版本信息注入。优先级：UPDATE_VERSION/UPDATE_COMMIT/UPDATE_DATE
// 环境变量（Makefile/CI 设置）→ git describe / rev-parse → 默认值。
use std::process::Command;

fn sh(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Some(s.trim().to_string())
}

fn main() {
    let version = std::env::var("UPDATE_VERSION")
        .ok()
        .or_else(|| sh(&["describe", "--tags", "--always", "--dirty"]))
        .unwrap_or_else(|| "dev".to_string());
    let commit = std::env::var("UPDATE_COMMIT")
        .ok()
        .or_else(|| sh(&["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "none".to_string());
    let date = std::env::var("UPDATE_DATE").unwrap_or_else(|_| "unknown".to_string());

    println!("cargo:rustc-env=UPDATE_VERSION={version}");
    println!("cargo:rustc-env=UPDATE_COMMIT={commit}");
    println!("cargo:rustc-env=UPDATE_DATE={date}");

    println!("cargo:rerun-if-env-changed=UPDATE_VERSION");
    println!("cargo:rerun-if-env-changed=UPDATE_COMMIT");
    println!("cargo:rerun-if-env-changed=UPDATE_DATE");
}
