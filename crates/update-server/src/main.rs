// updateserver 二进制入口（对应原 Go server/main.go）。
//
// 用法: updateserver [-addr :8080] [-dir ./package]
// 参数风格兼容 Go flag：支持 "-addr :8080" 与 "--addr=:8080" 两种写法；
// -h/--help 打印用法到 stderr 并退出 0；未知参数报错退出 2。
use std::net::TcpListener;
use std::sync::Arc;

const EXIT_USAGE: i32 = 2;

const USAGE: &str = "用法: updateserver [-addr :8080] [-dir ./package]
  -addr  监听地址（默认 :8080）
  -dir   分发数据目录（默认 ./package，布局 <dir>/package/<name>/<version>/）
  -h, --help  显示本帮助
";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut addr = ":8080".to_string();
    let mut dir = "./package".to_string();
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if a == "-h" || a == "--help" {
            eprint!("{}", USAGE);
            std::process::exit(0);
        }
        let body = a
            .strip_prefix("--")
            .or_else(|| a.strip_prefix('-'))
            .unwrap_or(a);
        if body == a {
            eprintln!("updateserver: unknown argument {a:?}");
            eprint!("{}", USAGE);
            std::process::exit(EXIT_USAGE);
        }
        if let Some((k, v)) = body.split_once('=') {
            if k == "addr" {
                addr = v.to_string();
            } else if k == "dir" {
                dir = v.to_string();
            } else {
                eprintln!("updateserver: unknown flag -{k}");
                eprint!("{}", USAGE);
                std::process::exit(EXIT_USAGE);
            }
        } else if body == "addr" || body == "dir" {
            i += 1;
            let v = match args.get(i) {
                Some(v) => v.clone(),
                None => {
                    eprintln!("updateserver: flag -{body} requires a value");
                    eprint!("{}", USAGE);
                    std::process::exit(EXIT_USAGE);
                }
            };
            if body == "addr" {
                addr = v;
            } else {
                dir = v;
            }
        } else {
            eprintln!("updateserver: unknown flag -{body}");
            eprint!("{}", USAGE);
            std::process::exit(EXIT_USAGE);
        }
        i += 1;
    }

    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("updateserver: bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("updateserver listening on {addr}, dir {dir}");

    let srv = Arc::new(update::server::Server::new(&dir));
    let handler = {
        let srv = srv.clone();
        Arc::new(move |req: &update::httpd::Request| srv.handle(req))
    };
    if let Err(e) = update::httpd::serve(listener, handler) {
        eprintln!("updateserver: serve: {e}");
        std::process::exit(1);
    }
}
