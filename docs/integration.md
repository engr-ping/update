# 宿主集成指南

本文档说明宿主应用（任意语言）如何集成 `update`。核心是三步：**分发二进制 → 准备配置 → 子进程调用并解析 stdout JSON**。

## 1. 集成原理

```
┌────────────────────────────┐
│  宿主应用（C/C++/Python/Go…）│
└────────────┬───────────────┘
             │ ① 子进程调用（参数入）
┌────────────▼───────────────┐
│   update 静态二进制          │
│  ② 读取配置 → 访问源 → 处理   │
└────────────┬───────────────┘
             │ ③ stdout=协议JSON / stderr=日志 / 退出码=分类
             ▼
     宿主解析 JSON，决定是否提示用户升级
```

- **语言无关**：宿主只做「拉起进程 + 读 stdout + 解析 JSON」，不需要链接任何库。
- **进程隔离**：`update` 崩溃不影响宿主；宿主可随时中断（进程被 kill 时已下载文件自动留在临时文件，不污染目标文件）。

## 2. 分发二进制

从 CI 产物（`.github/workflows/ci.yml` 的 artifacts）或 `make dist` 获取与宿主运行平台匹配的二进制：

| 平台 | 文件名 |
|---|---|
| Windows x64 | `update-<ver>-windows-amd64.exe` |
| Windows arm64 | `update-<ver>-windows-arm64.exe` |
| Linux x64 | `update-<ver>-linux-amd64` |
| Linux arm64 | `update-<ver>-linux-arm64` |
| macOS x64 | `update-<ver>-darwin-amd64` |
| macOS arm64 | `update-<ver>-darwin-arm64` |

二进制为静态链接、零依赖，随宿主应用一起分发即可（放到应用目录，路径可在配置/环境变量中指定）。

## 3. 准备配置文件

配置文件路径通过 `--config FILE` 传入，或设置环境变量 `UPDATE_CONFIG`。两种配置源示例见 `examples/`：

- `examples/github.json` — 公共 GitHub（token 通过环境变量注入，如 `GITHUB_TOKEN`）
- `examples/github-enterprise.json` — 内部 GitHub Enterprise（用户名+密码 Basic 认证）
- `examples/custom.json` — 自定义源（统一发布清单 + bearer/basic 认证 + 自定义头）

注意：**token 值绝不写在配置文件里**，配置只写环境变量名（`token_env` / `username_env` 字段），运行时从环境读取。

### 内部 GitHub（需要用户名密码）

内部 GitHub Enterprise 的 API 走 Basic 认证，配置 `api_base_url` 指向内网地址，并设置用户名与密码环境变量：

```json
{
  "source": {
    "type": "github-tag",
    "github": {
      "owner": "acme",
      "repo": "my-app",
      "api_base_url": "https://github.internal.example.com/api/v3",
      "username_env": "GHE_USERNAME",
      "token_env": "GHE_PASSWORD",
      "use_releases": true
    }
  }
}
```

```sh
export GHE_USERNAME=bob
export GHE_PASSWORD=my_password
./update check --config config.json
```

### GUI 登录：运行时传入凭据

如果宿主有登录对话框（GUI），拿到用户名密码后不必依赖环境变量，直接通过 `--username` / `--password` 传给 `update`（三个命令 check / download / list 均支持）：

```python
p = subprocess.run(
    ["update", "check", "--config", config_path,
     "--username", username, "--password", password],   # 来自 GUI 登录框
    capture_output=True, text=True, timeout=30,
)
```

规则：
- 传了 `--username` → 使用 **Basic 认证**（用户名 + 密码/token）
- 只传 `--password`（无用户名）→ 使用 **Bearer 认证**（token）
- 两者都不传 → 回退到配置里的 `token_env` / `username_env` 环境变量
- 注意：`--password` 会出现在进程参数中（`ps` 可见），公共网络环境建议优先用环境变量方式

## 4. 调用协议速查

| 命令 | 用途 | 成功退出码 |
|---|---|---|
| `update check --config F [--current-version X] [--platform os/arch]` | 检查是否有新版本 | 0 |
| `update download --config F --version X\|latest [--asset NAME] [--out PATH] [--platform os/arch]` | 下载产物到本地 | 0 |
| `update list --config F [--limit N] [--platform os/arch]` | 列出历史版本 | 0 |
| `update version` | 输出自身版本 | 0 |

**退出码**：`0` 成功 ｜ `2` 配置/用法错误 ｜ `3` 源错误（网络/HTTP/认证/解析）｜ `4` 下载错误（校验和/写盘）

**约定**：
- stdout 只含协议 JSON（`version` 子命令除外，输出纯文本版本号）
- 日志/错误一律走 stderr，宿主可显示给用户，不要解析
- 无更新也是退出码 0，用 JSON 里的 `update_available` 判断
- `--platform` 默认取宿主平台；`all` 表示不过滤（返回全部 assets）
- `--current-version` 覆盖配置里的 `current_version`

**check 输出示例**：

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
      {"name": "app-linux-amd64.tar.gz", "url": "https://...", "size": 12345}
    ]
  }
}
```

`assets` 已按目标平台过滤（或按 `product.asset_filter` 正则过滤）。

## 5. 各语言集成示例

### C

```c
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* 简单示例：仅判断是否有更新 */
int check_update(const char *config_path) {
    char cmd[512];
    snprintf(cmd, sizeof(cmd), "\"%s\" check --config \"%s\" > /tmp/update_result.json", UPDATE_BIN, config_path);
    int rc = system(cmd);
    if (rc != 0) return rc; /* 失败原因在 stderr */

    /* 解析 /tmp/update_result.json，读取 update_available、latest_version */
    FILE *f = fopen("/tmp/update_result.json", "r");
    /* ... 用你自己的 JSON 库解析（cJSON 等） ... */
    return 0;
}
```

### C++（Qt / std::system 均可）

```cpp
#include <QProcess>
#include <QJsonDocument>
#include <QJsonObject>

QJsonObject checkUpdate(const QString &configPath) {
    QProcess p;
    p.start("update", {"check", "--config", configPath});
    p.waitForFinished(30000);                 // 启动时同步检查，30s 超时
    QJsonObject out = QJsonDocument::fromJson(p.readAllStandardOutput()).object();
    return out;                               // {update_available, latest_version, release}
}
```

### Python

```python
import json
import subprocess

def check_update(config_path: str) -> dict:
    p = subprocess.run(
        ["update", "check", "--config", config_path],
        capture_output=True, text=True, timeout=30,
    )
    if p.returncode != 0:
        print("update check failed:", p.stderr)   # 日志在 stderr
        return {}
    return json.loads(p.stdout)

def download_update(config_path: str, version: str, out_path: str):
    p = subprocess.run(
        ["update", "download", "--config", config_path,
         "--version", version, "--out", out_path],
        capture_output=True, text=True, timeout=600,
    )
    if p.returncode != 0:
        raise RuntimeError(p.stderr)

result = check_update("config.json")
if result.get("update_available"):
    print(f"发现新版本 {result['latest_version']}")
    download_update("config.json", result["latest_version"], "./app_new.bin")
```

### Go

```go
package main

import (
	"encoding/json"
	"os/exec"
)

type CheckResult struct {
	UpdateAvailable bool `json:"update_available"`
	LatestVersion   string `json:"latest_version"`
}

func checkUpdate(cfgPath string) (*CheckResult, error) {
	out, err := exec.Command("update", "check", "--config", cfgPath).Output()
	if err != nil {
		return nil, err // stderr 已被合并进 err 的 ExitError
	}
	var r CheckResult
	if err := json.Unmarshal(out, &r); err != nil {
		return nil, err
	}
	return &r, nil
}
```

### Shell

```sh
if ./update check --config config.json | grep -q '"update_available":true'; then
    echo "有更新，开始下载"
    ./update download --config config.json --version latest --out ./new.bin
fi
```

## 6. 推荐集成流程（启动时检查一次）

```text
宿主启动
  │
  ├─ 检查 update 二进制是否存在（不存在则静默跳过）
  │
  ├─ update check --config config.json     （同步，超时建议 30s）
  │     ├─ 退出码 3/4 → 网络或源故障，静默或提示「检查更新失败」后继续启动
  │     └─ 退出码 0   → 解析 stdout JSON
  │
  ├─ update_available == false → 正常启动
  │
  ├─ update_available == true  → 提示用户（显示 latest_version + release.notes）
  │     └─ 用户同意 → update download --version latest --out <临时目录>
  │           └─ 退出码 0 → 校验文件（update 已内置 sha256 校验），
  │                         替换/重启由宿主自己完成（update 不做安装）
  │
  └─ 启动完成
```

要点：
- **下载不阻塞启动**：check 快速返回（一次 HTTP），download 可异步做。
- **安装由宿主负责**：`update` 只保证文件完整下载（sha256 校验 + 原子写盘），替换、备份、重启流程由宿主实现（v1 范围，见 `docs/design.md`）。
- **并发安全**：同一时刻只跑一个 `update` 实例；下载到临时目录可避免与正在运行的应用文件冲突。

## 7. 常见问题

- **为什么 check 无更新也返回 0？** 0 只表示「调用成功」，是否更新看 JSON 的 `update_available`；这样宿主不用拿退出码区分「没更新」和「出错了」。
- **为什么 stdout 是纯 JSON？** 方便任何语言的宿主直接 `json.loads(p.stdout)` 解析；所有人类可读信息走 stderr。
- **Windows 上怎么用？** 直接调用 `.exe`，参数一致；配置里注意 `download_url_template` 等 URL 的转义。
- **私有网络/内网源？** 用自定义源 + `versions_url` 指到内网 HTTP 地址即可；`http://` 允许（默认不校验 scheme，生产建议 HTTPS）。
