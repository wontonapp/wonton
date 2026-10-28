# ADR-0008 模型网关：magpie

- 状态：已接受（2026-10-28）

## 背景

用户要能自己设置 provider 和 key，而且只配一次就能让 Hermes 和各插件共用，不把 key 散落各处。
magpie（yetone/magpie，MIT，Go）是一个本地模型网关：同时支持 OpenAI Chat / Responses、
Anthropic Messages、Gemini 四种接口并能互相转换；内置 DeepSeek、Kimi、GLM、Qwen、豆包、OpenRouter、
OpenAI、Anthropic、Ollama 等预设；支持订阅账号、额度用尽自动切换和隐私脱敏；有 Docker 镜像和
Web UI；支持列表里有 Hermes Agent。

## 决策

- magpie 作为系统插件预装，锁定版本，可替换。
- 用户在 App 的“模型设置”小程序（magpie 自带的 Web UI）里配置 provider 和 key。
- 核心不自写模型网关，只做两件事：给声明了 `models` 权限的插件注入网关地址和只属于该插件的凭据；
  在网络层只允许这些插件访问 magpie。
- 本地模型插件（如 Ollama）装好后在 magpie 里注册为 provider。

## 理由

不造轮子；国内外主流 provider 和订阅账号现成支持，配置一处、处处可用。

## 代价与风险

- magpie 很新（2026 年 9 月创建）、迭代快、issue 多，必须锁版本，升级前回归。
- 订阅账号登录在容器场景下是否可用，待验证。
- 实时语音接口（WebSocket / WebRTC）不经过它，M2 的实时通话另行处理。

## 待验证（M0）

- 多个插件同时访问时，magpie 的鉴权方式（每插件独立凭据）。
- Hermes 指向 magpie 的配置方式。

## 验证结果（M0-4，2026-10-28）

环境：magpie 0.1.1327（stable 渠道）、Hermes Agent v0.22.5、Tuwunel v1.9.4，macOS + OrbStack。
内网的 npchy 拉不到 Docker Hub、也连不上海外 provider，所以没在它上面跑。复现：`deploy/dev/agent`
的 `dev.sh up` 再 `dev.sh smoke`。

**上游说明**：验证时容器里的 magpie 没有直接配 provider，而是加了一个 OpenAI 兼容的自定义 provider，
指向主人本机已在运行的 magpie（`host.docker.internal:3425`）。国内模型用 `rm/glm-5.3-flash` 跑日常测试；
海外只发了一次 Anthropic Messages 格式的短请求（`auto/claude-haiku-5-5`，经两层 magpie 转换，2 秒返回）。
生产环境没有这一层，用户在容器 magpie 的网页（“模型设置”）里直接配置 provider 和 key。

### 每插件独立凭据：用 magpie 的网关凭据（gateway key）

- 签发：核心为每个声明了 `models` 的插件生成一个随机值，在 magpie 容器里执行
  `magpie gateway-key add <插件 id> --key -`，值经标准输入传入；也可以让 magpie 生成。
  凭据以明文存在 magpie 数据目录的 `magpie/caller-keys.json`（权限 0600）。
- 网关必须监听容器网卡（`MAGPIE_ADDR=0.0.0.0:3425`）。这时来自其他容器的请求一律要带启用中的凭据
  （`Authorization: Bearer` 或 `x-api-key`），实测：
  - 不带凭据、带未签发的凭据：401，错误信息分别说明原因；
  - 新签发的凭据立即可用，`gateway-key remove` 后立即 401。magpie 每个请求都重读凭据文件，不用重启；
  - `gateway-key models <id> group/wonton-default` 之后，该凭据的 `/v1/models` 只列出这个路由组，
    点名其他模型返回 403，越权请求也记在该凭据名下；
  - 用量按凭据归属：`magpie usage --csv` 的 `caller_key_name` 就是插件 id。
- 另有每凭据的额度（按 token 或估算费用、按日/周/月或 N 小时，超额 429）、限定账号、
  来源地址白名单（`MAGPIE_ALLOW_IPS`），这次没用，核心以后可以在插件设置里暴露。
- 例外：magpie 容器内的回环请求不需要凭据（上游设计，供本机 Agent 用）。所以绝不能让别的服务共用
  magpie 的网络命名空间，核心网关也不能经回环把模型请求转给 magpie。
- 管理接口只有 CLI（经 Docker exec）是文档化的；网页用的 `/api/caller-keys/*` 靠网页密钥的 cookie 鉴权，
  属于内部接口。M1 先用 exec；需要更稳的接口时给上游提。

### Hermes 指向 magpie 的配置

- 模型：`config.yaml` 里定义 `providers.magpie`（`api: ${WONTON_MODEL_BASE_URL}`、
  `key_env: WONTON_MODEL_API_KEY`、`transport: chat_completions`、`discover_models: false`、`context_length`），
  再设 `model.provider: custom:magpie`、`model.default: ${WONTON_MODEL_DEFAULT}`。Hermes 会展开配置里的
  `${VAR}`，凭据只在环境变量里。Hermes 自己也会改写 `config.yaml`，所以由插件的 init 服务在首次启动前写入，
  之后不覆盖（见 `plugins/hermes/config.yaml`）。
- 默认模型指向 magpie 的路由组 `group/wonton-default`：换 provider 或模型只在 magpie 里改，
  Hermes 和其他插件不用动；路由组按顺序回退。
- 网关地址必须用带点的主机名（开发环境用网络别名 `magpie.wonton.internal`）。Hermes 把不带点的主机名
  当成本机模型服务，会去探测 `/api/tags`；magpie 正好兼容 Ollama 的这组接口，Hermes 就把它当成 Ollama，
  给每个请求加 Ollama 专用的 `options.num_ctx`，还按 Ollama 的运行窗口拒绝对话（实测）。
  核心注入 `WONTON_MODEL_BASE_URL` 时要遵守这一点。
- Matrix：环境变量只给 `MATRIX_HOMESERVER`、`MATRIX_USER_ID`、`MATRIX_ACCESS_TOKEN`（只给令牌，不给密码）、
  `MATRIX_ALLOWED_USERS`（主人）、`MATRIX_HOME_ROOM`（核心替主人建的私聊；注意不是插件清单里写的
  `MATRIX_HOME_CHANNEL`，没设时每个新会话的第一条回复都会插一段设置提示）。`MATRIX_E2EE_MODE=required`：
  Element Web 的加密私聊里多轮对话正常；Hermes 只凭令牌就自动建好了交叉签名，恢复密钥写在数据目录。
- 只收发消息：`display.platforms.matrix` 关掉工具进度、推理、中间消息和流式编辑，`display.busy_input_mode: queue`
  且 `busy_ack_enabled: false`，`platforms.matrix` 关掉表情回应、自动开线程和重启提示。关不掉的是“回复引用”：
  v0.22.5 的 Matrix 适配器没实现 `reply_to_mode`，每条回答都带 `m.in_reply_to`，由我们的客户端决定怎么显示。
- 健康检查：用 Hermes 自己生成的本机 API 密钥请求 `/health/detailed`，看 `platforms.matrix.state` 是否为
  `connected`。
- 开销：首轮请求约 1.3 万输入 token（系统提示和工具定义）；经 GLM-5.3-Flash 每轮 1 到 4 秒；
  空闲内存约 210 MB；镜像解压后 4.25 GB（arm64）。
- 本地模型：llama.cpp 跑的 Qwen2.5 1.5B 注册成 magpie 的自定义 provider 后，整条链路同样跑通
  （CPU 上首轮约 100 秒）。Hermes 要求至少 64K 窗口，本地模型服务要按此配置。

### 手机端（2026-10-28）

- iPhone 上的 Element X（iOS）在同一局域网用 `http://<本机 IP>:18008` 登录，接受明文 http，不用另套 https。
- 和 Hermes 的加密私聊三轮对话正常，回复分别在 2.3、3.3、5.2 秒后到达（GLM-5.3-Flash）。

### 结论

两项待验证都有了答案，决策不变：magpie 作为系统插件，每插件一个网关凭据，Hermes 只指向 magpie。
插件规范要补的地方（`matrix.bot` 的注入变量、`WONTON_MODEL_DEFAULT`、模型网关如何声明自己并签发凭据、
插件自有密钥、插件包文件、多 Bot 的凭据等）记在 M0-4 的 PR 里，由插件规范的修订统一处理。
