# update 通用软件更新模块 — 设计与实现方案

## 1. 目标与范围

一个可复用的升级/发布检查组件：**任何语言**开发的宿主项目、**任何平台**（Windows/Linux/macOS）都能调用。

**范围内**
- 检查是否有新版本（release 检查）
- 下载指定版本产物
- 列出历史版本（`update list`）
- GitHub tag/release 源、自定义源
- 需要认证的 GitHub 连接
- 宿主集成方式：**启动时单次同步检查**（CLI 进程短生命周期，一次调用即返回）

**范围外（v1 不实现）**
- 宿主进程内嵌（C ABI，后续单独引入）
- 自动安装/替换文件、服务自更新
- 增量/差分更新、签名校验（仅 checksum）

## 2. 关键决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 语言 | **Go**（已定） | stdlib 自带 HTTP/TLS；GitHub 认证零第三方依赖；三平台交叉编译一条命令；产物是零依赖静态二进制 |
| 形态 | **单静态 CLI 可执行文件**（已定） | 宿主通过子进程调用，任何语言都能集成；Go 侧靠接口形态实现语言无关，不依赖语言本身 |
| 宿主⇄CLI 协议 | **命令行参数入 + stdout JSON 出 + 退出码** | 通用、无共享内存/无 ABI 耦合，跨语言最简单 |
| 日志 | 一律走 **stderr** | 保证 stdout 只含协议 JSON，宿主可放心解析 |
| 版本比较 | **SemVer 2.0**，可退化为字典序 | GitHub tag 常见 `v1.2.3`，需要剥 `v` 前缀 |
| 宿主触发 | **启动时单次同步检查** | 用户确认的主要场景；CLI 设计为快速返回，宿主可阻塞也可异步拉起 |
| C ABI / DLL | **留口子，v1 不做** | 用户确认非硬需求；核心逻辑与 CLI 外壳解耦，后期以 `c-shared` 加一层壳即可 |

**备选方案（已评估）**
- Rust：静态二进制 + cdylib 都支持，但维护成本更高 → 备选。
- Python/C++：跨平台交付（依赖/Python 运行时、各 OS 构建链）成本高 → 不选。

## 3. 总体架构

```
┌────────────────────────────┐
│  Host 应用（任意语言）        │
└────────────┬───────────────┘
             │ 子进程调用
┌────────────▼───────────────┐
│  update CLI（Go 静态二进制）  │
│  ┌───────────────────────┐  │
│  │ 子命令分发 check/download│  │
│  └───────────────────────┘  │
│  ┌───────────┬───────────┐  │
│  │ github 源 │ custom 源  │  │
│  └───────────┴───────────┘  │
│  ┌───────────────────────┐  │
│  │ 传输层 HTTP             │  │
│  │ (认证/头/TLS/代理/超时)  │  │
│  └───────────────────────┘  │
└────────────┬───────────────┘
             │ HTTPS
     ┌───────▼────────┐
     │ GitHub API / 任意源 │
     └────────────────┘
```

数据流：`update check --config x.json` → 源按配置拉取发布清单 → 统一内部结构 → 版本比较 → stdout 输出 JSON。

## 4. CLI 契约

### 子命令

| 命令 | 作用 |
|---|---|
| `update check [--config FILE] [--current-version X] [--platform os/arch] [--username U] [--password P]` | 检查是否有新版本，stdout 输出结果 JSON |
| `update download [--config FILE] --version X [--asset NAME] [--out PATH] [--platform os/arch] [--username U] [--password P]` | 下载指定版本的匹配产物到本地 |
| `update list [--config FILE] [--limit N] [--platform os/arch] [--username U] [--password P]` | 列出历史版本（按时间/版本降序） |
| `update version` | 输出自身版本号 |
| `update help` | 用法说明 |

- `--current-version` 优先于 config 里的 `current_version`。
- `--platform` 默认自动探测；服务器给嵌入式/Linux 下载时显式覆盖。
- `--json` 显式要求 JSON 输出（默认就是 JSON，`--json` 仅为向后兼容保留）。
- `list` 输出：`{"schema":1,"versions":[{"version":"1.2.0","tag_name":"v1.2.0","published_at":"...","notes":"...","assets":[...]}]}`；`--limit` 默认 10。

### check 输出 JSON（schema v1）

```json
{
  "schema": 1,
  "check": {
    "current_version": "1.0.0",
    "latest_version": "1.2.0",
    "update_available": true
  },
  "release": {
    "version": "1.2.0",
    "tag_name": "v1.2.0",
    "published_at": "2024-01-15T10:00:00Z",
    "name": "v1.2.0",
    "notes": "bug fixes...",
    "assets": [
      {"name": "app-linux-amd64.tar.gz", "url": "https://...", "size": 12345}
    ]
  }
}
```

- 无更新时 `update_available=false`，`release` 仍返回最新版本信息（宿主可选展示）。
- `assets` 只包含**匹配当前平台**的产物；需要全部产物时宿主可用 `--platform all` 或后续 `update list`。

### 退出码

| 码 | 含义 |
|---|---|
| 0 | 成功（有/无更新都是成功，宿主看 JSON 判断） |
| 2 | 参数 / 配置错误 |
| 3 | 源错误（网络、超时、HTTP 4xx/5xx、认证失败） |
| 4 | 下载失败（校验和不匹配、写盘失败） |

## 5. 配置模型

配置文件为 JSON，路径通过 `--config` 传入；也可用 `UPDATE_CONFIG` 环境变量。token 只从环境变量读取。

### GitHub tag 源

```json
{
  "product": {
    "name": "my-app",
    "current_version": "1.0.0",
    "asset_filter": "app-{os}-{arch}.*"
  },
  "source": {
    "type": "github-tag",
    "github": {
      "owner": "acme",
      "repo": "my-app",
      "token_env": "GITHUB_TOKEN",
      "api_base_url": "https://api.github.com",
      "use_releases": true
    }
  }
}
```

- `use_releases: true`：走 `GET /repos/{owner}/{repo}/releases/latest`（有 release notes/assets）。
- `use_releases: false`：走 `GET /repos/{owner}/{repo}/tags`（纯 tag，无 assets）。
- `api_base_url` 可指向 GitHub Enterprise，实现「自定义连接」的一层；内部 GHE 需要用户名+密码时配置 `username_env`（触发 Basic 认证，token_env 作为密码/PAT）。
- `token_env`：值为环境变量名，如 `GITHUB_TOKEN`。未配置时按匿名请求，认证失败/限流时给出明确报错（匿名 60 次/h，认证 5000 次/h）。
- **运行时凭据接口**：三个子命令支持 `--username` / `--password` 覆盖配置凭据（GUI 登录场景）；传用户名 → Basic，仅传密码 → Bearer；都不传 → 环境变量。

### 自定义源

```json
{
  "source": {
    "type": "custom",
    "custom": {
      "versions_url": "https://updates.example.com/feed.json",
      "headers": {"X-Client": "my-app"},
      "auth": {"type": "bearer", "token_env": "UPDATE_TOKEN"},
      "download_url_template": "https://updates.example.com/{version}/{asset}"
    }
  }
}
```

- `versions_url` 返回**统一发布清单**（见 §6），自定义源服务端只需吐一个 JSON。
- `download_url_template` 可选；为空时直接用清单里的 `assets[].url`。
- `auth` 支持 `bearer`/`basic`；头信息任意自定义 → 覆盖「自定义连接 + 认证」需求。

## 6. 统一发布清单

自定义源返回的 feed 格式（GitHub 源内部也归一化到同一结构）：

```json
{
  "version": "1.2.0",
  "published_at": "2024-01-15T10:00:00Z",
  "name": "v1.2.0",
  "notes": "release notes",
  "checksum": "sha256:...",
  "assets": [
    {"name": "app-linux-amd64.tar.gz", "url": "https://.../app-linux-amd64.tar.gz", "size": 12345},
    {"name": "app-windows-amd64.zip",  "url": "https://.../app-windows-amd64.zip",  "size": 67890}
  ]
}
```

- 源接口（Go 内部）：
  ```go
  type Source interface {
      Latest(ctx context.Context) (*Release, error) // 最新发布
      List(ctx context.Context, limit int) ([]*Release, error) // 历史版本（list 命令用）
  }
  ```
- `checksum` 为可选整体校验值；若 assets 元素带 `sha256` 字段则逐个校验。

## 7. 版本与平台匹配

- **版本比较**：SemVer 2.0（`golang.org/x/mod/semver`，stdlib 生态），自动剥 `v`/`release-` 前缀；非 SemVer 标签按字典序降序取最新。
- **平台匹配**：默认 `runtime.GOOS/GOARCH`；候选产物按「文件名包含 os + arch」或 `asset_filter` 正则匹配；`all` 返回全部。
- 匹配优先级：`{os}-{arch}` → `{os}` → 无平台标识（视为通用）。

## 8. 分发服务器（updateserver）

配套的只读分发服务器，把产物目录发布为 §6 格式的 feed，客户端 `custom` 源可直接消费。

### 8.1 数据目录布局

```
package/<name>/<version>/<file>
package/<name>/<version>/meta.json   # 可选
```

- `<name>` / `<version>` 均为目录名；`<version>` 目录不可变（不提供删除/覆盖接口）。
- 版本目录按 semver 降序排列（复用 `internal/version` 的比较逻辑），非 SemVer 目录按字典序降序。
- 每个版本目录可选 `meta.json` 覆盖默认元数据：`name` / `notes` / `published_at` / `checksum` / `assets.{name}.{sha256,size}`。
- 默认 `published_at` 回退为版本目录的修改时间（UTC RFC3339）。

### 8.2 端点

| 端点 | 行为 |
| --- | --- |
| `GET /feed/<name>.json` | 该软件的发布清单数组（§6 结构，新版本在前）；无此产品 → 404；名称含路径穿越 → 400 |
| `GET /feeds.json` | `{"feeds":[{"name","latest_version","versions"}]}` |
| `GET /package/<name>/<version>/<file>` | 产物下载；路径穿越/不存在 → 404 |
| `GET /healthz` | `{"status":"ok"}` |

- feed 中每个产物的 `url` 按请求的 scheme/host 自动生成，支持反代 `X-Forwarded-Proto: https`，客户端无需 `download_url_template`。
- 下载做防御性校验：`filepath.EvalSymlinks` 解析后必须落在 `<dir>/package/<name>` 之下，防止符号链接逃逸。
- 认证：下载匿名；如需鉴权建议在前置反代（nginx/caddy）完成，服务器保持无状态。
- 目录扫描（`ReadDir`）在每次请求时执行，无缓存，保证新版本即时可见。

### 8.3 使用

```sh
./bin/updateserver -addr :8080 -dir ./package
```

客户端配置：

```json
{
  "product": {"name": "my-app", "current_version": "1.0.0"},
  "source": {"type": "custom", "custom": {"versions_url": "https://updates.example.com/feed/my-app.json"}}
}
```

部署（systemd / 反代 / 鉴权 / 发布流程）见 `docs/deployment.md`。

## 9. 安全

- token/密码只经环境变量注入，**永不写入仓库、不出现在任何输出/错误信息中**。
- 传输默认 HTTPS；HTTP 仅当显式配置允许且打警告到 stderr。
- 提供整体或逐文件 `sha256` 校验，校验失败 → 退出码 4。
- 下载到临时文件后原子重命名，避免半成品。

## 10. 目录结构

```
update/
├── go.mod
├── Makefile                # build / test / vet / 三平台交叉编译
├── .gitignore
├── README.md               # 集成示例 + 分发服务器用法
├── cmd/
│   └── update/
│       └── main.go
├── server/                 # 分发服务器（updateserver）
│   ├── main.go             # 入口与路由
│   ├── server.go           # feed 生成、下载、目录防护
│   └── server_test.go
├── internal/
│   ├── config/             # 配置加载/校验/默认值
│   ├── version/            # semver 比较、标签解析、平台匹配
│   ├── source/             # Source 接口 + github/custom 实现
│   ├── transport/          # HTTP 客户端（认证/头/TLS/代理/超时/错误归一化）
│   ├── cli/                # 子命令、JSON 输出、退出码
│   └── versioninfo/        # 自身版本信息
└── docs/
    ├── design.md           # 本文档
    └── integration.md      # 宿主集成指南
```

## 11. 实现计划

### Phase 1 — 脚手架（可运行空 CLI）
- `go mod init`、`cmd/update/main.go`（子命令骨架）、Makefile（含 `build`/`test`/`vet`/交叉编译 `dist`）、`.gitignore`。
- 验收：三平台交叉编译出二进制，`update version` 正常。

### Phase 2 — 配置与版本
- `config` 包：加载/校验/默认值，环境变量注入 token。
- `version` 包：标签解析、semver 比较、平台匹配。
- 验收：单测覆盖配置错误与版本比较用例。

### Phase 3 — 源与传输
- `Source` 接口 + GitHub 实现（releases/tags，Enterprise base url，认证）。
- 自定义源实现（统一发布清单 + 模板下载 URL）。
- `transport` 包：超时、TLS、代理、认证头、错误归一化。
- 验收：用 `net/http/httptest` 模拟 GitHub API 与自定义源，跑通端到端。

### Phase 4 — CLI 命令
- `check` / `download` / `list` / `version`，JSON 输出 + 退出码，日志走 stderr。
- 验收：`update check` 输出 §4 协议 JSON；错误路径返回正确退出码。

### Phase 5 — 加固与文档
- 下载校验和、原子写盘；README 集成示例（C/Python/Go/Shell）。
- 验收：`make test` 全绿，`make dist` 产出 win/linux/mac 三平台产物。

### Phase 6 — 后续扩展（不做进 v1）
- C ABI `c-shared` 共享库（进程内嵌入）。
- 增量更新、GPG/签名校验、UI 托管组件。

## 12. 已决策清单

- 语言：**Go**；形态：**单静态 CLI 可执行文件**（子进程调用）。
- 宿主触发：启动时单次同步检查；`check` 快速返回。
- v1 范围：`check` + `download` + `list` + `version`，GitHub/release 与自定义源，认证 GitHub。
- DLL/C ABI：留口子，v1 不实现；核心与 CLI 解耦便于后补。
- 下载进度：v1 默认下载完成才返回（宿主异步等待进程结束即可）。
- 分发服务器：只读静态分发（无上传 API），匿名下载，feed 与 §6 统一发布清单完全一致；鉴权交给前置反代。
- 服务器元数据：每个版本目录可选 `meta.json`，缺失字段回退文件系统默认值。
