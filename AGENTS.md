# Update 通用软件更新模块

## 构建产物

```sh
make build        # 构建当前平台客户端二进制 -> bin/update（cargo build --release）
make server       # 构建分发服务器 -> bin/updateserver
make test         # 运行全部测试（cargo test --workspace）
make vet          # cargo clippy 静态检查（--all-targets -D warnings）
make fmt          # cargo fmt --check（未装 rustfmt 时提示跳过）
make dist         # 客户端交叉编译三平台 6 种产物 -> dist/
make dist-server  # 服务器交叉编译三平台 6 种产物 -> dist/
make lib          # 当前平台 C ABI 共享库 -> dist/libupdate.{so,dylib,dll} + dist/libupdate.h
```

## 运行

`update check` 从配置读取 GitHub tag/release 或自定义源，检查是否有新版本：

```sh
./bin/update check --config config.json
```

输出为 JSON，stdout 只输出协议 JSON，日志/错误走 stderr，退出码见 `docs/design.md`。

## 架构

- 四个子命令：`check` / `download` / `list` / `version`（`crates/update-core/src/cli.rs`，外壳 `crates/update-cli`）。
- 两个源：`github-tag`（releases + tags，`github.rs`）与 `custom`（统一发布清单 feed，`custom.rs`），均在 `crates/update-core/src/source/`。
- 分发服务器：`crates/update-server/`（updateserver）只读静态分发，目录布局 `<dir>/package/<name>/<version>/`，生成与 custom 源完全一致的 feed（`/feed/<name>.json`），复用 `crates/update-core` 的 semver 做版本排序；可选 `meta.json` 增强元数据。
- 配置与协议：`docs/design.md`（README 也有宿主集成示例）；宿主集成指南：`docs/integration.md`。
- CICD：`.github/workflows/ci.yml`（clippy + test + 客户端/服务器三平台 6 产物交叉编译）。
- C ABI：`crates/update-lib`（cdylib）已实现，复用 `run_to_buffers`；核心与 CLI 外壳解耦。

## 测试

- 全部测试：`make test`
- 单个包：`cargo test -p update-core --lib <名称过滤>`
- 端到端：`cargo test -p update-core --lib cli::`（使用内置 TestServer 模拟源，不需要真实网络）

## 约定

- **零第三方依赖**：所有功能基于 Go stdlib（HTTP/TLS/JSON/semver 比较均为手写实现）。
- **交叉编译用 rustup target + 显式链接器**（linux 需 gcc 交叉链接器；windows/macos 需 rustup target），保持静态链接，否则会破坏三平台产物。
- token 等凭据只从环境变量注入或 `--username`/`--password` 运行时参数传入，绝不硬编码到代码或配置文件。
- 认证规则：`username_env`/`--username` 存在 → Basic 认证；仅 token → Bearer。
- 文档以中文为主。
