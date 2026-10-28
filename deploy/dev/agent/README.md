# M0-4 开发环境：默认 Bot

Tuwunel（不联邦）+ magpie 模型网关 + Hermes（以 Matrix bot 身份接入）。`dev.sh` 扮演核心：
生成密钥、注册主人和 Bot 的账号、建好主人和 Bot 的加密私聊、给 Hermes 签发 Matrix 令牌和
magpie 网关凭据。Hermes 的配置和插件草稿共用 [`plugins/hermes/config.yaml`](../../../plugins/hermes/config.yaml)。
验证结论见 [ADR-0008](../../../docs/adr/0008-model-gateway-magpie.md) 的“验证结果”。

```text
手机 / Element ──Matrix──▶ Tuwunel ◀──Matrix── Hermes ──网关凭据──▶ magpie ──▶ 上游 provider
                                   (matrix 网络)          (models 内部网络)   (egress 网络)
```

## 用法

```bash
cp .env.example .env   # 填模型上游：UPSTREAM_URL、UPSTREAM_KEY、UPSTREAM_MODELS
./dev.sh up            # 生成密钥和账号，配置 magpie，启动并等到全部健康
./dev.sh smoke         # magpie 鉴权检查 + 和 Bot 的两轮对话，全部通过才返回 0
./dev.sh web           # 打印 magpie 网页（模型设置）的登录链接，可在里面直接加 provider
./dev.sh down          # 停止，保留数据；./dev.sh reset 连数据和 .secrets/ 一起删
```

- 没有模型上游时，在 `.env` 里设 `WONTON_AGENT_LOCAL_MODEL=1`，用 CPU 上的 Qwen2.5 1.5B 跑通链路
  （首次下载约 1 GB，每轮首条回复要一两分钟）。
- 改了 `.env` 里的上游或模型后运行 `./dev.sh providers`。
- `plugins/hermes/config.yaml` 只在数据卷里没有 `config.yaml` 时写入；改了它要 `./dev.sh reset`。

## 用手机或其他客户端测试

主人账号是 `owner`，密码在 `.secrets/owner.env`；登录后会看到和 Hermes 的加密私聊。

- 服务器地址填 `http://<本机局域网 IP>:18008`（端口用 `WONTON_AGENT_MATRIX_PORT` 改）。
  Tuwunel 支持 Element X 要求的 Simplified Sliding Sync。
- 有的客户端拒绝明文 http。这时可以在本机用 `tailscale serve` 之类给 18008 套上 https，
  手机加入同一个 tailnet 后访问。
- 浏览器客户端（如自己起的 Element Web）要通过 `localhost` 访问，加密功能才可用。
