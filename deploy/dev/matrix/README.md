# 本地 Matrix 服务器（Tuwunel）

本地开发用的 Tuwunel，配置 `tuwunel.toml` 和 `crates/wonton-matrix` 的集成测试共用。和产品一致：
关闭联邦，不开放注册，不接受密码登录，账号只能用核心签发的 JWT 登录（ADR-0009 的验证结果）。

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
cargo test -p wonton-matrix --test tuwunel
```

每个测试用 Docker 起一台一次性的 Tuwunel，镜像和配置取自本目录。testcontainers 不读 docker context，
Docker 套接字不在 `/var/run/docker.sock`（如 OrbStack、Colima）时先设置 `DOCKER_HOST`：

```bash
export DOCKER_HOST="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
```

消息测试也能连现成的服务器，例如 `dev.sh up` 起的服务器，或经隧道的客户端本地代理：

```bash
WONTON_MATRIX_HOMESERVER=http://127.0.0.1:8008 cargo test -p wonton-matrix --test tuwunel messaging
```

签发私钥默认取 `.secrets/jwt-ed25519.pem`，用 `WONTON_MATRIX_JWT_KEY` 指定别的路径。
