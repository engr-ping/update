# Update 通用软件更新模块

## 构建产物

```sh
make build     # 构建当前平台二进制 -> bin/update
make test      # 运行全部测试
make vet       # go vet 静态检查
make dist      # 交叉编译三平台 6 种产物 -> dist/
```

## 运行

`update check` 从配置读取 GitHub tag/release 或自定义源，检查是否有新版本：

```sh
./bin/update check --config config.json
```

输出为 JSON，stdout 只输出协议 JSON，日志/错误走 stderr，退出码见 `docs/design.md`。

## 架构

- 四个子命令：`check` / `download` / `list` / `version`（`internal/cli/`）。
- 两个源：`github-tag`（releases + tags，`internal/source/github.go`）与 `custom`（统一发布清单 feed，`internal/source/custom.go`）。
- 配置与协议：`docs/design.md`（README 也有宿主集成示例）；宿主集成指南：`docs/integration.md`。
- CICD：`.github/workflows/ci.yml`（vet + test + 三平台 6 产物交叉编译）。
- DLL/C ABI 留口子未实现；核心（internal/）与 CLI 外壳解耦，便于后补。

## 测试

- 全部测试：`make test`
- 单个包：`go test ./internal/version/...`
- 端到端：`go test ./internal/cli/...`（使用 `net/http/httptest` 模拟源，不需要真实网络）

## 约定

- **零第三方依赖**：所有功能基于 Go stdlib（HTTP/TLS/JSON/semver 比较均为手写实现）。
- **交叉编译必须 CGO_ENABLED=0**，保持静态链接，否则会破坏三平台产物。
- token 等凭据只从环境变量注入或 `--username`/`--password` 运行时参数传入，绝不硬编码到代码或配置文件。
- 认证规则：`username_env`/`--username` 存在 → Basic 认证；仅 token → Bearer。
- 文档以中文为主。
