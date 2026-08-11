// update — 通用软件更新模块核心逻辑（Rust 重写版）。
//
// 结构对应原 Go 版 internal/ 各包：
//   - semver / match:  semver 2.0 比较、标签解析、平台/产物匹配（原 internal/version）
//   - config:          配置加载/校验/默认值，凭据只从环境变量注入（原 internal/config）
//   - transport:       HTTP 客户端（认证/头/TLS/代理/超时/错误归一化/下载校验+原子写盘）
//                      （原 internal/transport）
//   - source:          源抽象 + GitHub / custom 实现（原 internal/source）
//   - httpd:           最小 HTTP/1.1 服务器（纯 std，替代 Go net/http 的服务端部分）
//   - updateserver:    只读分发服务器（原 server/）
//   - cli:             子命令、JSON 协议输出、退出码（原 internal/cli）
//   - versioninfo:     构建期版本注入（原 internal/versioninfo）
//
// 宿主 ⇄ CLI 契约不变：参数入 + stdout 协议 JSON + stderr 日志 + 退出码分类。
pub mod cli;
pub mod config;
pub mod custom;
pub mod github;
pub mod httpd;
#[path = "match.rs"]
pub mod r#match;
pub mod semver;
pub mod server;
pub mod source;
pub mod transport;
pub mod versioninfo;

#[cfg(test)]
pub mod testutil;
