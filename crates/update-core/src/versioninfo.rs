// 构建期版本元数据，由 build.rs 注入（对应原 Go internal/versioninfo）。
pub const VERSION: &str = env!("UPDATE_VERSION");
pub const COMMIT: &str = env!("UPDATE_COMMIT");
pub const DATE: &str = env!("UPDATE_DATE");

/// update 二进制自身的版本字符串。
pub fn version_string() -> String {
    VERSION.to_string()
}
