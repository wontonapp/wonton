# M0-4 开发环境：默认 Bot

> 核心已经接入 Matrix Bot 和模型网关（插件规范实现计划第 7、8 项）：Hermes 按插件包装进 `deploy/dev/core`，由核心
> 代它登录、签发 magpie 凭据，M0-4 的检查（网关鉴权、两轮对话、用量归属）在 `deploy/dev/core/smoke.sh` 里跑。
> 这套环境里 `dev.sh` 扮演核心的部分只作对照；手机用密码登录 Tuwunel、CPU 上的本地模型这些实验还在这里做。

Tuwunel（不联邦）+ magpie 模型网关 + Hermes（以 Matrix bot 身份接入）。`dev.sh` 扮演核心：
生成密钥、注册主人和 Bot 的账号、建好主人和 Bot 的加密私聊、给 Hermes 签发 Matrix 令牌和
magpie 网关凭据。Hermes 的配置和 Wonton 插件来自 [`plugins/system.lock`](../../../plugins/system.lock) 锁定的插件包。
验证结论见 [ADR-0008](../../../docs/adr/0008-model-gateway-magpie.md) 的“验证结果”。

```text
手机 / Element ──Matrix──▶ Tuwunel ◀──Matrix── Hermes ──网关凭据──▶ magpie ──▶ 上游 provider
                                   (matrix 网络)          (models 内部网络)   (egress 网络)
```

## 用法

先按 [`deploy/dev/core`](../core/README.md) 的方式构建核心镜像（`wonton-core:dev`）。`up` 通过该镜像运行
`plugin fetch-locked /system.lock /output/packages`，挂载仓库锁文件和临时输出目录，核对提交与包摘要后把结果放进
`.secrets/packages/`；compose 只读挂载其中 `default-plugins/hermes/` 的配置和 `wonton/` 文件。
取包失败即停止，不替换已缓存的包。可在 `.env` 里用 `WONTON_AGENT_CORE_IMAGE` 指定包含该命令的其他核心镜像。

```bash
cp .env.example .env   # 按需改模型上游：UPSTREAM_URL、UPSTREAM_KEY、UPSTREAM_MODELS（见下）
./dev.sh up            # 按锁文件取包，生成密钥和账号，配置 magpie，启动并等到全部健康
./dev.sh smoke         # magpie 鉴权检查 + 和 Bot 的两轮对话，全部通过才返回 0
./dev.sh web           # 打印 magpie 网页（模型设置）的登录链接，可在里面直接加 provider
./dev.sh down          # 停止，保留数据；./dev.sh reset 连数据和 .secrets/ 一起删
```

- 示例配置的上游是开发机上已经在跑的 magpie，容器经 `host.docker.internal` 访问它。设了 `UPSTREAM_URL`
  就不能留空 `UPSTREAM_KEY`（留空时 `up` 在配置模型这一步直接退出）；示例里的 `placeholder` 只在这一种情况下可以原样用：
  OrbStack 转发进来的连接在宿主机 magpie 看来是本机连接，不校验 key，填什么都行。上游换成别的 provider，
  或在 Linux 上经网桥访问宿主机 magpie 时，要填真的 API key 或网关凭据。
- 没有模型上游时，在 `.env` 里设 `WONTON_AGENT_LOCAL_MODEL=1`，用 CPU 上的 Qwen2.5 1.5B 跑通链路
  （首次下载约 1 GB，每轮首条回复要一两分钟）。
- 改了 `.env` 里的上游或模型后运行 `./dev.sh providers`。
- 锁定包的 `config.yaml` 只在数据卷里没有该文件时写入；改锁文件后运行 `./dev.sh up` 重新取包，已有配置要 `./dev.sh reset` 才重置。
- `smoke` 会临时签发一个名为 `smoke-temp` 的网关凭据，验证“签发立即可用、吊销立即失效”。magpie 允许重名凭据，
  所以按签发时新增的 id 吊销，脚本中断或失败退出时也会吊销。早先中断的冒烟可能留下同名凭据：在 magpie
  容器里用 `magpie gateway-key list` 找到，再 `magpie gateway-key remove <id>` 清掉。

## 用手机或其他客户端测试

主人账号是 `owner`，密码在 `.secrets/owner.env`；登录后会看到和 Hermes 的加密私聊。

- 服务器地址填 `http://<本机局域网 IP>:18008`（端口用 `WONTON_AGENT_MATRIX_PORT` 改）。
  Tuwunel 支持 Element X 要求的 Simplified Sliding Sync。
- 有的客户端拒绝明文 http。这时可以在本机用 `tailscale serve` 之类给 18008 套上 https，
  手机加入同一个 tailnet 后访问。
- 浏览器客户端（如自己起的 Element Web）要通过 `localhost` 访问，加密功能才可用。

## 同一台机器上再起一套环境

项目名（默认 `wonton-dev-agent`）决定数据卷和网络的名字；`.secrets/` 则属于检出目录。所以第二套环境要放在
另一个检出目录里（比如验证改动用的 git worktree），在它自己的 `.env` 里换项目名和两个端口：

```bash
WONTON_AGENT_PROJECT=wonton-dev-agent-feat-x
WONTON_AGENT_MATRIX_PORT=18108
WONTON_AGENT_MAGPIE_WEB_PORT=13440
```

也可以只在命令行（或 shell 里 `export`）设这三个变量：环境里已经设置的变量优先于 `.env`，和 docker compose 一致。
`dev.sh` 的每条命令（包括 `reset`）都先读配置，只作用在这个项目上，不会动另一套的容器、数据卷和端口。
不换项目名的话，两个检出目录会共用同一批数据卷，却各有各的 `.secrets/`，见下一节。

## 排查

**`up` 报“登录失败”，并说 `.secrets/` 和数据卷不是同一套**：Tuwunel 的数据卷里已经有 `owner`、`hermes`
账号，但 `.secrets/` 里记的密码登录不上。通常是 `.secrets/` 重新生成了（换了检出目录，或只删了 `.secrets/`），
数据卷却还在。二选一：

- 数据不要了：先 `./dev.sh reset`（删除这个项目的数据卷和 `.secrets/`，聊天记录会丢，已登录的客户端要重新登录），
  再 `./dev.sh up`。
- 数据要留：把原来那份 `.secrets/` 放回来；或者让这个检出目录用自己的项目名和端口（见上一节），不去动原来那套。
