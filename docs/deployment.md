# 部署文档

本文档覆盖 `updateserver` 分发服务器的部署与日常发布流程，以及客户端的对接方式。所有命令以 Linux 为例，Windows/macOS 等价（二进制见 `make dist-server` 产物）。

## 1. 快速开始

```sh
# 1. 构建（或从 CI artifacts / Release 下载对应平台二进制）
make server

# 2. 创建产物目录，放入首个版本
mkdir -p package/my-app/v1.0.0
cp app-linux-amd64.tar.gz package/my-app/v1.0.0/

# 3. 启动
./bin/updateserver -addr :8080 -dir ./package
```

验证：

```sh
curl http://127.0.0.1:8080/healthz            # {"status":"ok"}
curl http://127.0.0.1:8080/feed/my-app.json   # 发布清单，新版本在前
curl http://127.0.0.1:8080/feeds.json         # 全部软件列表
curl -o app.bin http://127.0.0.1:8080/package/my-app/v1.0.0/app-linux-amd64.tar.gz
```

## 2. 目录布局

```
package/                         # 服务器数据目录（-dir 指向）
└── <软件名>/                    # 一个软件一个目录
    ├── <版本号>/                # 版本目录，发布后不可变
    │   ├── app-linux-amd64.tar.gz
    │   ├── app-windows-amd64.zip
    │   └── meta.json            # 可选：增强元数据
    └── v1.0.0/                  # 同一软件可有多版本，自动按 semver 排序
```

`meta.json` 可选字段：

```json
{
  "name": "My App",
  "notes": "修复了 X",
  "published_at": "2024-02-01T00:00:00Z",
  "checksum": "overall-checksum",
  "assets": {
    "app-linux-amd64.tar.gz": { "sha256": "abc123...", "size": 12345 }
  }
}
```

缺失时 `published_at` 回退为版本目录的修改时间。**建议发布时显式写 `meta.json`**，使 feed 输出确定、不依赖文件系统时间戳。

### 2.1 软件包 vs 脚本/数据包

更新内容不限于软件二进制——**项目脚本、平台包、数据包等任何可分发内容都按同一套机制处理**（`<软件名>` 换成内容名即可）：

```
package/
├── my-app/                      # 软件二进制
│   └── v1.2.0/app-linux-amd64.tar.gz
└── vinfast_demo/                # 项目脚本/平台包（zip 整体分发）
    ├── v1.0.0/vinfast_demo.zip
    └── v1.1.0/
        ├── vinfast_demo.zip
        └── meta.json            # {"name":"vinfast_demo","notes":"...","published_at":"..."}
```

脚本包与软件唯一的差异是产物形态（zip vs 各平台二进制），客户端流程完全一致：

```json
{
  "product": { "name": "vinfast_demo", "current_version": "1.0.0" },
  "source": { "type": "custom", "custom": { "versions_url": "https://updates.example.com/feed/vinfast_demo.json" } }
}
```

```sh
./bin/update check   --config client.json     # 有新版本 → 提示升级
./bin/update download --config client.json --version latest --out ./vinfast_demo.zip
```

对比旧 Python 版仓库服务（`repo_server_data/packages/xxx.zip` + 同名 json、单版本覆盖）：新布局支持**多版本并存、semver 自动排序、sha256 校验、统一 feed 协议**，发布新脚本包时新增版本目录即可，旧版本仍可下载（回滚友好）。

## 3. 发布新版本（checklist）

1. 更新代码 → 交叉编译 → 打 tag → CI 产出 `update-*` 与 `updateserver-*` 三平台产物。
2. 在服务器上创建 `<dir>/package/<name>/<new-version>/`，放入该版本全部平台产物。
3. 写 `meta.json`（name/notes/published_at/checksum/assets.sha256）。
4. **不要覆盖或删除已发布的版本目录**——旧客户端可能仍在下载；需要回滚时发布新版本号，而不是修改旧目录。
5. 无缓存、无需重启：服务器每次请求实时扫描目录，新版本立即在 `/feed/<name>.json` 可见。

## 4. systemd 部署（Linux）

`/etc/systemd/system/updateserver.service`：

```ini
[Unit]
Description=Update distribution server
After=network.target

[Service]
ExecStart=/opt/update/bin/updateserver -addr 127.0.0.1:8080 -dir /opt/update/package
Restart=always
RestartSec=3
User=www-data

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now updateserver
sudo systemctl status updateserver
```

## 5. 反向代理 + HTTPS（nginx）

服务器只监听内网地址（见上例 `127.0.0.1:8080`），由 nginx 终止 TLS 并转发。feed 中产物 URL 会根据 `X-Forwarded-Proto: https` 自动生成 https 链接。

```nginx
server {
    listen 443 ssl;
    server_name updates.example.com;

    ssl_certificate     /etc/letsencrypt/live/updates.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/updates.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 300s;    # 大文件下载
    }
}
```

认证（如果需要）也在反代层完成，例如 basic auth：

```nginx
location / {
    auth_basic "Updates";
    auth_basic_user_file /etc/nginx/updates.htpasswd;
    proxy_pass http://127.0.0.1:8080;
    ...
}
```

客户端对应配置（basic 认证 + 反代）：

```json
{
  "product": { "name": "my-app", "current_version": "1.0.0" },
  "source": {
    "type": "custom",
    "custom": {
      "versions_url": "https://updates.example.com/feed/my-app.json",
      "auth": { "type": "basic", "username_env": "U_USER", "token_env": "U_PASS" }
    }
  }
}
```

注意：凭据只写环境变量名（`U_USER`/`U_PASS`），运行时从环境读取；也可用 `--username`/`--password` 运行时覆盖（GUI 登录场景）。

## 6. 容器部署（可选）

```dockerfile
FROM scratch
COPY updateserver /updateserver
COPY package /package
EXPOSE 8080
ENTRYPOINT ["/updateserver", "-addr", ":8080", "-dir", "/package"]
```

产物目录建议挂载持久卷；镜像本身无状态。

## 7. 常见问题

| 现象 | 处理 |
| --- | --- |
| feed 404 | 检查软件名与目录名是否一致；`/feed/<name>.json` 中 name 不含 `.json` |
| 下载 404 | 版本目录或文件名不匹配；符号链接目标越界会被拒绝 |
| 新版本不出现 | 服务器无缓存，实时扫描；确认目录名是合法版本号且位于 `package/<name>/` 下 |
| 需要 HTTPS | 见 §5 反代方案（服务器本身不支持 TLS，推荐反代） |
| 需要鉴权 | 反代层做（见 §5）；服务器保持无状态匿名 |
| 客户端下载很慢 | 大文件建议反代层开启缓存/限速，`proxy_read_timeout` 调大 |
