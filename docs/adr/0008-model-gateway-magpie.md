# ADR-0008 模型网关：magpie

- 状态：已接受（2026-10-28）

## 背景

用户要能自己设置 provider 和 key，而且只配一次就能让 Hermes 和各插件共用，不把 key 散落各处。
magpie（yetone/magpie，MIT，Go）是一个本地模型网关：同时支持 OpenAI Chat / Responses、
Anthropic Messages、Gemini 四种接口并能互相转换；内置 DeepSeek、Kimi、GLM、Qwen、豆包、OpenRouter、
OpenAI、Anthropic、Ollama 等预设；支持订阅账号、额度用尽自动切换和隐私脱敏；有 Docker 镜像和
Web UI；支持列表里有 Hermes Agent。

## 决策

- magpie 作为系统插件预装，锁定版本。主人换不掉；可替换只表示我们换实现时 Agent 不用改（主人 2026-11-04 定）。
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

## 验证结果（M1，核心接入模型网关）

核心按插件规范的“模型网关”接上 magpie，决策不变。环境：magpie 0.1.1327、macOS + OrbStack。可复跑：
`deploy/dev/core/smoke.sh`（“模型网关”一步）；签发、验证、对账、吊销和注入的各种情况在假网关上测
（`cargo test -p wonton-core models`）。

- 预装：magpie 是系统插件，随开发用的核心镜像放在 `/usr/share/wonton/system-plugins/magpie`，核心启动后在后台
  装上（`WONTON_SYSTEM_PLUGINS`），镜像已在本机时两秒内装好并通过健康检查。“模型设置”网页由网关代登录（见下面的
  “模型设置网页”一节）；provider 也可以用命令行配置：`docker exec <magpie 容器> magpie provider add …`。
- 凭据：核心在 magpie 容器里执行 `sh /wonton/credentials.sh issue <插件 id>`，值走标准输入；
  `magpie gateway-key list` 里出现以插件 id 命名、不限模型的凭据。用它请求 `GET /v1/models` 返回 200
  （没配 provider 时是空列表），不带凭据或带错的凭据 401。
- 转发：使用方拿到 `WONTON_MODEL_BASE_URL=http://models.wonton.internal:<网关端口>/v1`、
  `WONTON_MODEL_DEFAULT=group/wonton-default`。把 magpie 的上游指向主人本机的 magpie（同 M0-4）后，流式补全经核心
  逐块转发，首 token 约 2.1 秒（GLM-5.3-Flash）；`magpie usage --csv` 的 `caller_key_name` 是使用方的插件 id。
- 对账：在 magpie 里删掉这个凭据后请求得到 401；重启核心，核心用记下的同一个值重新签发，使用方不用重启就恢复 200。
- 吊销：卸载使用方后，magpie 里没有这个凭据了，旧值得到 401。
- 隔离：没申请 `models` 的插件从自己的网络里访问 `models.wonton.internal` 得到 403 `permission_denied`。

## 验证结果（M1，核心安装 Hermes）

M0-4 的检查改由核心完成，复跑：`deploy/dev/core/smoke.sh` 的“Hermes”一步。环境同上，Hermes Agent v0.22.5。

- 安装：`dev.sh install plugins/hermes` 把插件包目录打成 tar 上传（带 `config.yaml`），核心代 Bot 登录、建主房间、
  签发凭据 `com.nousresearch.hermes/default`（Bot 服务用 Bot 的凭据，见插件规范“模型网关”），8 秒装好并通过健康
  检查（Matrix 已连上）。Hermes 只凭注入的变量工作，`config.yaml` 没为 Wonton 改过。
- 对话：主人的一台新设备经隧道在加密私聊里问两轮，两轮都收到加密回复，第二轮记得第一轮的问题，两轮共约 8.6 秒
  （GLM-5.3-Flash）。
- 用量：`magpie usage` 里三次请求都记在 `com.nousresearch.hermes/default` 名下，第一轮约 1.3 万输入 token，首 token
  1.3 到 3.2 秒，同 M0-4。
- 卸载（删除数据）：Bot 的设备吊销、账号停用，它的 Matrix 令牌得到 401；magpie 里没有它的凭据了。

## 验证结果（M1，“模型设置”网页）

magpie 的网页经网关代登录（ADR-0009 的“网关代登录”）打开，主人不用输网页密钥，浏览器和 App 也拿不到它。可复跑：
`deploy/dev/core/smoke.sh` 的“模型设置”一步。环境：magpie 0.1.1327。

- 鉴权：magpie 的网页认名为 `magpie_web_<端口>` 的 Cookie，值就是 `MAGPIE_WEB_KEY`；不带或带错时页面和 `/api/*`
  都得到 401（纯文本）。同名的 Cookie 里有一个对就放行，所以挡住浏览器伪造的是网关先删掉同名 Cookie（行为测试覆盖）。
- 网页密钥是核心生成的插件密钥 `web_key`（32 字节），以 `MAGPIE_WEB_KEY` 注入容器，由网关代加进 Cookie。页面、脚本和
  `/api/settings`、`/api/providers` 等接口的响应里都没有它（实测）。
- magpie 自己的登录链接 `/?k=<网页密钥>` 回 303 和一个带密钥、400 天有效的 `Set-Cookie`：网关去掉这个 `Set-Cookie`，
  冒烟里验证过。
- 网页的修改是同源的 `fetch`（`POST /api/…`，JSON），经网关照常转发：在网页上关掉、再打开上游，`magpie providers`
  里随即可见。别的插件网页发来的同样请求得到 401，到不了 magpie。

## 补充：默认模型（2026-10-30）

主人在“模型设置”里加任意一家 provider 的 key，默认 Bot 就能回答，不用再做别的（GOAL 的 M1 第 3 项）。

- **谁管**：magpie 插件自己。插件里的常驻服务 default-model（`plugins/magpie/default-model.sh`）维护 `wonton-default`
  这个组，维护失败时它的健康检查不过，插件显示异常。`default_model` 是网关插件声明的，由它保证存在、选得合理；核心只
  把它注入给使用方，不认识 magpie 的组（ADR-0015）。
- **规则**：用 magpie 自己的“Cheap first”模板挑出的两个模型，便宜的在前、最强的在后：请求先发给便宜的，它出错才换
  最强的，并记住出错的模型（`routing=order`）。贵贱和强弱是 magpie 按 models.dev 的模型目录（价格、能力）判断的，
  模型目录里没有的模型不算数。挑不出来时（还没有 provider、只有一个模型、最强的旁边没有够便宜的，或者 magpie 还没
  下载到模型目录），用全部模型（通配 `*`），按 provider 添加的先后、每家列出模型的顺序排。provider、模型列表或模型
  目录变了，default-model 两秒内重新挑。组里只放这两个，不再接上其余的模型：两个都出错时把错误交给 Bot，不一路试到
  又贵又不对的模型上去。“全部模型”指 magpie 用的那些：主人给哪家挑了模型，就只有挑的那些；没挑、服务商列出的又超过
  24 个时，只有前 24 个。模板和 `*` 都只在这些里面挑、展开。
- **自定义接入**：App 明确指定了模型时，`custom-save` 让新服务商只暴露这个模型，并把选择交给默认组维护器；未被主人
  修改的默认组只匹配这个 `provider/*`，不再从模板或全局 `*` 里另挑，也避免模型名被组的逗号或 reasoning 语法解释。
  服务商删除后恢复模板规则。主人已修改的组不覆盖，自定义服务商仍会保存，默认组继续使用主人的选择；要切换可在
  “模型设置”里改组。
- **看得到、改得了**：“模型设置”的路由页里有 wonton-default 这张卡片，按顺序列出成员，排第一的就是现在用的；每次
  请求实际由哪个模型回答，用量里看得到。主人在网页（或命令行）里改了这个组的任何地方，default-model 就不再动它；
  删掉它，或者把它的 id 改成别的，就按上面的规则重建一个 id 是 wonton-default 的组，主人的组不动。
- **和主人同时改**：default-model 写组之前再读一次，挑模型的这段时间里主人改了这个组就不写，此后当作主人改过；写的
  时候只按 id 改这一个组。magpie 改 `providers.json` 是把整个文件读出、改完、写回，不加锁（0.1.1327 实测，网页和命令
  行都是），两边的保存相隔不到约 0.1 秒时，后写的一方会盖掉先写的一方，这要 magpie 加锁才能消除。default-model 只在
  provider、模型列表或模型目录变了之后写，平时不写。
- **没有模型时**：组是空的，magpie 的模型列表里没有它，请求它得到 404，报错很长，绕过了 Hermes 对模型报错的改写，
  原样进了聊天。所以 Hermes 的插件包带一个 Hermes 插件（`plugins/hermes/wonton`，`post_gateway_admission` 钩子）：
  每条消息先查一次网关的模型列表，默认模型不在里面时直接回提示，不调模型，这条消息也不进会话历史：
  - 一个模型都没有：请在“模型设置”里添加模型服务商和它的 API key；
  - 有模型、默认模型却不可用（组被改空了）：请检查 wonton-default 路由组。

  网关连不上、回答不对时不拦，交给 Hermes 照常报错。主人加了 key，下一条消息就照常回答，不用重启。

没选的做法：

- 全部模型（`*`）按顺序：顺序是服务商列出的顺序，不看价格。OpenAI 预设下第一个是 2023 年的 gpt-4-0613（每百万
  token 输入 30、输出 60 美元），组里还有 tts-1、davinci-002 这类不能对话的模型；OpenRouter 下第一个是
  openrouter/auto（由 OpenRouter 按请求另挑模型，价格不定），通义千问下是旗舰 qwen-max（实测见下）。
- 自己按模型名挑（如名字里带 mini、flash 的排前面）：命名没有规律可循，会过时；magpie 的模板用的是价格。通配也排不了
  序：通配匹配到的模型一律按 magpie 列出模型的顺序排，写在前面的通配不会排在前面。
- 用模板建一次就不管（magpie 网页里点一下模板就是这样）：模板建的组记的是具体模型，删掉 provider 后就空了，加了更
  便宜的也不换。
- 核心在网关装好后经 `docker exec` 建组：核心就要认识 magpie 的命令和组，换网关时跟着改。

### 验证结果（2026-10-30）

环境：magpie 0.1.1327、Hermes Agent v0.22.5，其余同“核心接入模型网关”一节。可复跑：`deploy/dev/core/smoke.sh` 的
“Hermes”一步。

- 挑出的模型：服务商的模型列表来自假的上游（和真实列表一样长、一样乱，key 是假的），模型目录是 magpie 从 models.dev
  下载的：

  | 服务商（预设） | 列出 / 留下 | 全部模型按顺序时的第一个 | “Cheap first” |
  | --- | --- | --- | --- |
  | OpenAI | 41 / 27 | gpt-4-0613 | gpt-5-mini → gpt-5 |
  | OpenRouter | 22 / 22 | openrouter/auto | z-ai/glm-4.6 → anthropic/claude-sonnet-4.5 |
  | 通义千问（qwen） | 26 / 21 | qwen-max | qwen-flash → qwen3-max |
  | 阿里云百炼（bailian） | 26 / 21 | qwen-max | qwen-plus → qwen-max |
  | DeepSeek | 2 / 2 | deepseek-chat | 挑不出来（两个一样贵），用全部模型：deepseek-chat 在前 |
  | Anthropic | 20 / 20 | claude-haiku-5-5 | claude-haiku-5-5 → claude-sonnet-5-5 |
  | Gemini（google） | 40 / 26 | gemini-2.0-flash | gemini-2.5-flash → gemini-4-argon |
  | 智谱（zhipu） | 25 / 23 | glm-4-plus | glm-4.7-flash → glm-5.3-flashx |
  | Kimi（moonshot） | 6 / 6 | kimi-k3.1-pro | 挑不出来（K3.1 Pro 旁边没有够便宜的），用全部模型：kimi-k3.1-pro 在前 |

  “留下”是 magpie 去掉嵌入、图片、语音、审核等模型后的个数；OpenAI 留下 27 个，只用前 24 个（gpt-5-pro、gpt-5-codex、
  chatgpt-4o-latest 不在里面）。请求 `group/wonton-default` 由 gpt-5-mini 回答（之前的 `*` 由 gpt-4-0613 回答）。
  后四家的列表按各家文档和 models.dev 拼出，没有 key，没对过真实接口返回的顺序：Anthropic 从新到旧，Gemini 从旧到新、
  混着嵌入、语音、图片和视频模型，智谱从旧到新，Kimi 是它文档上现有的 6 个（moonshot-v1 系列已在 2026-08 下线）。
  留下的里面还有不能对话的（Gemini 的 aqa、veo、lyria，智谱的 cogvideox-3），模板不挑它们：把 models.dev 标价为 0
  的 lyria 选进来，Gemini 挑出的不变。Kimi 最便宜的 K2.6 约是 K3.1 Pro 一半的价，magpie 不算它便宜；用全部模型时
  第一个是 Kimi 列出的第一个，现有的 6 个都是对话模型，窗口至少 256K。
- 全部模型：Gemini 留下 26 个，`*` 展开成前 24 个；主人多选一个（`magpie provider models google +<模型>`），`*` 就是
  25 个；只选 3 个，`*` 就是这 3 个，模板也只在这 3 个里挑。
- 模型目录：magpie 的服务进程启动时下载它，命令行不下载。断网的 default-model 用服务进程下载好的目录，挑出的结果和
  联网时一样。没有目录时模板照样挑，但不看价格（OpenAI 预设下是 gpt-3.5-turbo → gpt-4-0613），所以 default-model 先用
  `*`，用的是上表第三列的模型；目录放进去后两秒内换成 gpt-5-mini → gpt-5。
- 维护：先后加 OpenAI、DeepSeek，删 OpenAI，组依次是 `*`、gpt-5-mini → gpt-5、不变、deepseek-chat → deepseek-reasoner；
  主人改成手动只用 deepseek-reasoner 后再加 OpenAI，组不变；主人删掉组，两秒内按规则重建成 gpt-5-mini → gpt-5。加删
  provider 时 magpie 不改组在 `providers.json` 里的定义，default-model 据此认出主人改没改过。组里没有成员时
  `GET /v1/models` 不列出 `group/wonton-default`，有成员时列出。default-model 第一次维护完 5 秒内就是 healthy，
  `docker stop` 时立刻退出。
- 和主人同时改：default-model 和 magpie 的网页服务共用一个配置目录，主人的操作经网页用的接口（`/api/groups/save`、
  `/api/groups/delete`）发出。主人在网页上另建一个组，default-model 的组不变；default-model 挑模型时（让
  `group template` 停住）主人在网页上改了这个组，放行 6 秒后还是主人的版本，日志记一次“主人改过”，再加 provider 也
  不动它；主人删掉这个组，或者把它的 id 改成别的、名字不变，default-model 都重建 `wonton-default`，主人的组不动，也不
  多出别的组。本机 OrbStack 上挑一次模型 60 到 85 毫秒，`group set` 写一次 80 到 90 毫秒。
- 网页：路由页按 `/api/groups` 画组的卡片，这个组在里面，带着成员和路由方式；卡片可以编辑、排序、删除，手动路由时
  选中的那个带着标记。“路由组”功能默认开着。
- 冒烟：还没有 provider 时网关的模型列表是空的，给 Hermes 发“你好”，回的就是“一个模型都没有”的提示。只执行
  `magpie provider add`、不动组（经开发机上的 magpie，只给一个模型 GLM-5.3-Flash，挑不出来，组里是 `*`），网关的
  模型列表里就有了 `group/wonton-default`，Hermes 在同一个私聊里接着对话两轮，第二轮记得第一轮的问题。
- 钩子：在 Hermes 镜像里对着假的模型列表直接调用钩子：列表为空回第一种提示，有别的模型、没有默认模型回第二种；列表里
  有默认模型、消息以 `/` 开头、网关回 500 或连不上时都不拦。按 Hermes v0.22.5 的源码，钩子回 `handled` 时这条消息算
  处理过了，不写进会话。
