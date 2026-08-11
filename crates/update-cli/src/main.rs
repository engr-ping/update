// update CLI 二进制入口（对应原 Go cmd/update/main.go）。
//
// 收敛 std::env::args 后交给 update-core 的 cli::run，由它完成子命令
// 分发、JSON 协议输出与退出码分类；此处只转发退出码。
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rc = update::cli::run(&args);
    std::process::exit(rc);
}
