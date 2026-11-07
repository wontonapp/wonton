# 本地 Matrix 服务器（Tuwunel）

本地开发用的 Tuwunel。基础配置 `tuwunel.toml` 和 `crates/wonton-matrix` 的集成测试共用，保持最小：
不开放注册，不接受密码登录，账号只能用核心签发的 JWT 登录（ADR-0009 的验证结果）。联邦选项单独放在
`federation.toml`（第二份 `-c` 配置，ADR-0050），只有这个栈的 compose 挂载——测试在 127.0.0.1 上起
Tuwunel，联邦开着会被 Tuwunel 的配置检查拒绝。`dev.sh up` 还会生成联邦用的自签证书（ADR-0050）。

```bash
deploy/dev/matrix/dev.sh up            # 首次生成核心的 Ed25519 签发密钥，启动并等到健康
deploy/dev/matrix/dev.sh login alice   # 签发 JWT 并登录一台新设备，输出 access_token
deploy/dev/matrix/dev.sh jwt alice     # 只签发 JWT（2 分钟有效）
deploy/dev/matrix/dev.sh down          # 停止；reset 连数据一起删除
```

- 地址 `http://127.0.0.1:8008`，只监听本机；端口用 `WONTON_DEV_MATRIX_PORT` 改。
- 签发私钥在 `.secrets/`，不提交；签发需要 OpenSSL 3 及以上。

## 测试

```bash
cargo test -p wonton-matrix --test tuwunel -- --include-ignored
```

每个测试用 Docker 起一台一次性的 Tuwunel，镜像和配置取自本目录。这些测试标了 `#[ignore]`，所以要加
`--include-ignored`，不加则只列出、不运行；`scripts/check.sh` 在有 Docker 时自动带上，CI 的“Rust Docker 测试”作业
也会运行。testcontainers 不读 docker context，
Docker 套接字不在 `/var/run/docker.sock`（如 OrbStack、Colima）时先设置 `DOCKER_HOST`：

```bash
export DOCKER_HOST="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
```

消息测试也能连现成的服务器，例如 `dev.sh up` 起的服务器，或经隧道的客户端本地代理：

```bash
WONTON_MATRIX_HOMESERVER=http://127.0.0.1:8008 cargo test -p wonton-matrix --test tuwunel -- --include-ignored messaging
```

签发私钥默认取 `.secrets/jwt-ed25519.pem`，用 `WONTON_MATRIX_JWT_KEY` 指定别的路径。

### 经隧道

用 `crates/wonton-tunnel` 的示例在 Tuwunel 前面接一条隧道（结果见 ADR-0003 的验证结果）。三个命令各开一个
终端，客户端可以在另一台机器上，`--addr` 给服务端的直连地址：

```bash
cargo run -p wonton-tunnel --example tunnel-client -- id     # 输出客户端 endpoint ID
```

```bash
cargo run -p wonton-tunnel --example tunnel-server -- --port 47100 --upstream 127.0.0.1:8008 --allow <客户端 ID>
```

```bash
cargo run -p wonton-tunnel --example tunnel-client -- proxy --server <服务端 ID> --addr 127.0.0.1:47100 --listen 127.0.0.1:18008
```

然后让测试经本地代理访问：

```bash
WONTON_MATRIX_HOMESERVER=http://127.0.0.1:18008 cargo test -p wonton-matrix --test tuwunel -- --include-ignored messaging
```
