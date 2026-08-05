# update — 通用软件更新模块

一个语言无关的软件更新模块：宿主应用（任意语言、Windows/Linux/macOS）通过子进程调用 `update` 检查新版本并下载产物。

- **语言无关**：CLI 子进程调用 + stdout JSON，任何语言零 ABI 耦合。
- **零第三方依赖**：纯 Go stdlib 实现，产物是静态二进制。
- **多源**：GitHub tag/release（含认证、GitHub Enterprise）与自定义 HTTP 源。
- **安全**：token 只从环境变量注入；下载带 sha256 校验；原子写盘。

## 构建

```sh
make build     # 当前平台二进制 -> bin/update
make test      # 全部测试
make vet       # go vet
make dist      # 三平台 6 种产物 -> dist/
```

## 使用

```sh
# 检查是否有新版本
./bin/update check --config config.json

# 下载最新版本匹配当前平台的产物
./bin/update download --config config.json --version latest --out ./app.tar.gz

# 列出历史版本
./bin/update list --config config.json --limit 5
```

stdout 只输出协议 JSON，日志/错误走 stderr。退出码：`0` 成功、`2` 配置/用法错误、`3` 源错误、`4` 下载错误。

## 配置

### GitHub 源（tag / release）

```json
{
  "product": {
    "name": "my-app",
    "current_version": "1.0.0"
  },
  "source": {
    "type": "github-tag",
    "github": {
      "owner": "acme",
      "repo": "my-app",
      "token_env": "GITHUB_TOKEN",
      "use_releases": true
    }
  }
}
```

- `use_releases: true` 使用 GitHub Releases（含下载产物 assets）；`false` 只读 tag（无 assets）。
- 认证方式二选一（或二选二）：
  - `token_env`：环境变量名，运行时读取其值作为 Bearer token（`Authorization: Bearer <token>`），匿名请求有 60 次/h 限流。
  - `username_env` + `token_env`：**HTTP Basic 认证**（用户名+密码/PAT），用于需要用户名密码的内部 GitHub Enterprise（`api_base_url` 指向 `https://<内部域名>/api/v3`）。见 `examples/github-enterprise.json`。
- `api_base_url` 可指向 GitHub Enterprise，实现自定义连接。

**GUI 登录场景**：宿主程序可在运行时用 `--username` / `--password` 覆盖配置中的凭据（例如从登录对话框拿到用户名密码后直接传入），无需写环境变量：

```sh
./bin/update check --config config.json --username bob --password s3cret
```

`--username` 表示 Basic 认证；只传 `--password` 表示 Bearer token。

### 自定义源

自定义源只需一个 HTTP 端点返回发布清单（单个对象或数组，新版本在前）：

```json
{
  "source": {
    "type": "custom",
    "custom": {
      "versions_url": "https://updates.example.com/feed.json",
      "headers": { "X-Client": "my-app" },
      "auth": { "type": "bearer", "token_env": "UPDATE_TOKEN" },
      "download_url_template": "https://updates.example.com/files/{version}/{asset}"
    }
  }
}
```

发布清单格式：

```json
{
  "version": "1.2.0",
  "published_at": "2024-01-15T10:00:00Z",
  "notes": "release notes",
  "assets": [
    {
      "name": "app-linux-amd64.tar.gz",
      "url": "https://updates.example.com/app-linux-amd64.tar.gz",
      "size": 12345,
      "sha256": "abc123..."
    }
  ]
}
```

`auth` 支持 `bearer` 或 `basic`；`download_url_template` 支持 `{version}` / `{asset}` 占位符，资产缺少 `url` 时自动填充。

## 输出协议

`check` 输出：

```json
{
  "schema": 1,
  "current_version": "1.0.0",
  "latest_version": "1.2.0",
  "update_available": true,
  "release": {
    "version": "1.2.0",
    "tag_name": "v1.2.0",
    "published_at": "2024-01-15T10:00:00Z",
    "name": "v1.2.0",
    "notes": "bug fixes",
    "assets": [
      { "name": "app-linux-amd64.tar.gz", "url": "https://...", "size": 12345 }
    ]
  }
}
```

`download` 输出：`{"schema":1,"version":"1.2.0","file":"app.tar.gz"}`
`list` 输出：`{"schema":1,"versions":[...]}`

## 宿主集成

完整集成指南见 **`docs/integration.md`**（分发、协议、C/C++/Python/Go/Shell 示例、推荐流程、FAQ）。

核心三步：分发对应平台的静态二进制 → 准备 `config.json`（token 走环境变量）→ 子进程调用并解析 stdout JSON：

```python
import json, subprocess
p = subprocess.run(["update", "check", "--config", "config.json"], capture_output=True, text=True)
result = json.loads(p.stdout)          # {"update_available": true, "latest_version": "1.2.0", ...}
if result["update_available"]:
    subprocess.run(["update", "download", "--config", "config.json",
                    "--version", "latest", "--out", "./app_new.bin"], check=True)
```

C/C++、Go、Shell 示例与启动时检查推荐流程见集成指南。

## 测试

所有测试基于 `net/http/httptest` 模拟源，不需要真实网络：

```sh
make test
```

## 目录结构

```
cmd/update/       CLI 入口
internal/cli/     子命令、JSON 输出、退出码
internal/config/  配置加载与校验
internal/source/  源抽象：GitHub / custom
internal/transport/ HTTP 客户端
internal/version/ semver 比较、平台匹配
internal/versioninfo/ 构建期版本注入
docs/design.md    完整设计文档
```
