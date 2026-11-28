# ADR-0129 在 App 里原生接模型：Agent 插件声明模型接入命令，核心转发，壳用 App 内浏览器登录和取 key；默认 Agent 叫云吞

- 状态：已接受（2026-11-27，主人当天的反馈起草，取舍主人同日定，见“主人已定”）；2026-11-28 修订第 3 节的设备码、回跳和
  Android 浏览器（实现时的复审）
- 取代：[ADR-0034](0034-onboarding-and-help.md) 2026-11-19 修订里“模型一步只剩‘沿用当前模型’，登录和填 key 在上游自己的设置页里做”；
  “接模型”一节打开插件网页的做法
- 补充：[ADR-0108](0108-hermes-default-kernel.md) 第 4 节（模型仍只走 Hermes 自带的供应商接入，本篇只加 App 侧的入口和插件侧的合同）；
  [ADR-0110](0110-one-hermes-container-per-person.md) 第 2 节的插件命令（同一种形状再加一条）；[ADR-0126](0126-agent-list-bar-and-cloud.md)
  的云朵头像（默认 Agent 的名字和头像在第一次配置时就能改）
- 关联：[ADR-0015](0015-small-components-standard-interfaces.md)、[ADR-0044](0044-bot-profile-and-status-avatars.md)、
  [ADR-0064](0064-input-secrets.md)、[ADR-0077](0077-plugin-settings.md)、[ADR-0088](0088-family-accounts.md)、
  [ADR-0120](0120-dumb-shell-pages.md)、[ADR-0122](0122-primary-agent-and-profiles.md)

## 背景

主人 2026-11-27 在 iPhone 上试新版后的原话：

> 现在配置模型居然是跳转 Hermes 的 dashboard 进行配置？？？绝对不行。我们只是基于 Hermes，但是要做很多体验优化的。
> 模型配置应该 app 这边直接可以配置。而且有非常丝滑的利用 app 内的浏览器网页打开效果可以跳转对应 provider 之类的进行登录，
> 密钥获取之类的。而且默认的 Agent 名字头像都应该是云吞的。配置的时候就要有一个头像选择这种。

引导的“接模型”一步和聊天里“还没有可用的模型”的提示都把人送进 Hermes 插件网页的 Models 页（插件清单 `webui.path: /models`，
核心在没有模型网关时退回默认 Bot 插件的网页）。那是 Hermes 给电脑浏览器做的控制台：英文、表格密、术语多，填 key 要先找到
Keys 页，订阅登录要在控制台里切页签。

[ADR-0108](0108-hermes-default-kernel.md) 定了模型只走 Hermes 自带的供应商接入、不另造 provider 目录和登录适配，但没定 App 的
交互。本篇保留这条边界：服务商有哪些、key 怎么存、登录怎么换令牌，仍然全由 Hermes 做；Wonton 只把“列服务商、填 key、登录、
选模型、测一下”这几件事做成原生页面，经一个不提 Hermes 的插件合同拿数据。

### Hermes v0.23.4 已有的能力

插件用的官方镜像是 `nousresearch/hermes-agent:v0.23.4`。它的控制台后端已经把模型设置做成了 HTTP 接口，都能按 `profile` 参数
限定到某个 profile（ADR-0110 里一个 Bot 就是一个 profile）。以下读自上游 `v0.23.4` 标签的源码：

| 能力 | 上游接口 | 位置 |
| --- | --- | --- |
| 列出每个服务商要的 key、是否已填、尾号预览、取 key 的网址 | `GET /api/env` | `hermes_cli/web_routers/config_env.py` |
| 保存、删除 key（写这个 profile 的 `.env`，顺带清掉旧副本） | `PUT`、`DELETE /api/env` | 同上 |
| 保存前试 key（OpenRouter、OpenAI、xAI、Gemini 有探针，其他回“没法验证”） | `POST /api/providers/validate` | 同上 |
| 自定义 OpenAI 兼容地址：试连、列模型 | `POST /api/providers/validate`（`OPENAI_BASE_URL`）、`/api/providers/custom-endpoints` | 同上 |
| 能登录的账号和状态 | `GET /api/providers/oauth` | `hermes_cli/web_routers/oauth.py` |
| 设备码登录：开始、轮询、取消 | `POST …/{id}/start`、`GET …/{id}/poll/{session}`、`DELETE …/sessions/{session}` | 同上 |
| 已登录、已填 key 的服务商及其模型 | `GET /api/model/options` | `hermes_cli/web_routers/models.py` |
| 新接好的服务商推荐的默认模型 | `GET /api/model/recommended-default` | 同上 |
| 设主模型（贵的模型先要确认） | `POST /api/model/set` | 同上 |

- 控制台里能完成的登录只有设备码一种：Nous Portal、ChatGPT/Codex 订阅、xAI Grok、MiniMax，外加声明了设备码登录的
  provider profile。Anthropic 账号、Claude Code、Qwen CLI 是 `external`，要在终端跑命令；上游注释写明不在控制台里替 Claude 订阅
  换令牌是因为 Anthropic 的 OAuth 使用政策。
- OpenRouter 另有一条 OAuth PKCE 流程（`hermes_cli/auth_openrouter.py`）：浏览器授权后带 `code` 回跳，拿 `code` 换回一把
  普通的 API key。上游只在命令行里用，回跳地址是本机回环端口。
- 控制台的写接口要会话令牌（`X-Hermes-Session-Token`）。令牌默认每次启动随机生成、只注入网页；设
  `HERMES_DASHBOARD_SESSION_TOKEN=file:<路径>` 时从文件读（`hermes_cli/web_server_session_token.py`），插件可以借这个让容器里的
  其他进程拿到同一个令牌。
- 设备码登录的会话放在控制台进程的内存里，15 分钟过期，由控制台的后台线程轮询服务商并保存凭据。

## 决策

### 1. 合同：Agent 插件声明一条模型接入命令

`kind: agent` 的插件可以在清单的 `agent` 下声明 `models`，形状同 ADR-0110 的 Bot 命令和模型网关的凭据命令：核心经 Docker exec
在指定服务的容器里执行，参数固定，输入是标准输入上的一行 JSON，输出是标准输出上的一行 JSON。

```yaml
agent:
  protocol: matrix
  models:
    service: gateway        # 在哪个服务里执行；有 bots 的插件是这个 Bot 所在那一组的这个服务
    command: [/opt/hermes/.venv/bin/python, /opt/data/plugins/wonton/models.py]
```

每次调用的输入都带 `op`、`bot`（ADR-0110 的 Bot 名，也就是 profile 名）和 `languages`（App 的语言，插件据此写人看的那一句），
其余字段按操作：

| `op` | 输入 | 输出 |
| --- | --- | --- |
| `catalog` | 无 | `providers`：每个服务商的 `id`、`name`、`recommended`（插件建议放在前面的）、`auth`（下面三种的列表）、`connected`（填了 key 或登录了）、`account`（尾号或账号名，可空）；`current`：主模型的 `provider` 和 `model`，没选时为空 |
| `save_key` | `provider`、`key`，自定义地址时另带 `base_url` | `check`：`accepted`、`rejected` 或 `unverified`；`rejected` 时不保存 |
| `disconnect` | `provider` | 无 |
| `login_start` | `provider`，`browser_login` 时另带 `redirect_uri` | `session`、`url`，设备码另有 `code`；`expires_in`、`interval`（秒） |
| `login_poll` | `session` | `state`：`pending`、`approved`、`denied`、`expired`、`failed`，加可选的一句 `message` |
| `login_finish` | `session`、`callback_url`（只给 `browser_login`） | 同 `login_poll` |
| `login_cancel` | `session` | 无 |
| `models` | `provider` | `models`：`id`、`name`（可空）、`recommended` |
| `select` | `provider`、`model`，确认过贵模型时另带 `confirmed: true` | `current`；要确认时回 `confirm` 和一句说明，不保存 |
| `test` | 无 | `result`：`ok`、`rejected`、`unreachable`、`upstream`，加可选的 `message` 和耗时 |

`auth` 的三种：

- `api_key`：`key_url`（去哪拿 key，可空）、`key_prefix`（key 的固定开头，如 `sk-or-`，给壳判断剪贴板，可空）、`base_url`
  （`required` 或 `optional`，带默认值；只有自定义服务商要）。
- `device_login`：设备码登录，`login_start` 给网址和码，App 轮询 `login_poll`。
- `browser_login`：浏览器授权后回跳 App，`login_start` 用 App 给的 `redirect_uri` 拼授权网址，App 拿回跳网址调 `login_finish`，
  插件换凭据。

退出码：`0` 成功；`2` 输入不合法（标准输出带 `field` 和一句说明）；`3` 这个服务商或会话插件不认识；`4` Agent 自己还没起来，
接口不通；其余一律算上游出错，核心带上截短的标准错误如实报。服务商拒了 key、登录被拒或过期、测试不通都不是命令失败，
退出码是 `0`，结果在输出的字段里。

合同只说“这件事要哪些数据”，不说界面怎么摆。服务商有哪些、叫什么、先后顺序、取 key 的网址、能不能登录、推荐哪个模型，
全由插件按它的上游给；核心和 App 里没有服务商名单。

### 2. 核心：只转发、认人、记审计，不存 key

- 客户端核心经隧道调核心的按 Bot 路由（`/_wonton/api/bots/<插件 ID>/<Bot 名>/models/…`，每个操作一条，具体形状在接口 PR 里定），
  核心找到这个 Bot 所在的容器、执行插件声明的命令，按协议类型解析输出再返回。插件没声明 `agent.models` 时返回
  `unsupported`，App 不显示模型入口。
- 谁能改：有 ADR-0088 的 `model_settings` 权限，并且这个 Bot 归他（私人 Bot）或他能管共享 Bot（`server_settings`）。其他人
  返回 `forbidden`。只看不改（`catalog`、`models`）同样要这两条：别人 Bot 的服务商和账号名也不给看。
- key 只出现在 `save_key` 的请求体和命令的标准输入里。核心不落盘、不写日志、不进审计；命令的标准输出和标准错误在记日志、
  回给 App 之前，先把这次提交的 key 原文替换成占位。审计只记谁、哪个 Bot、什么操作、哪个服务商。
- 每次执行有超时：`test` 60 秒，其余 30 秒。超时如实报上游出错，不重试。轮询的节奏由客户端核心按插件给的 `interval` 定，
  核心不长挂请求。
- 错误按 [AGENTS.md](../../AGENTS.md) 的五类给：连不上家里（客户端核心判断）、`forbidden`、`not_installed` 或 `not_ready`
  （插件没装，容器没在跑，或命令退出 `4`）、`upstream`（其余失败，带截短的详情）、`invalid_input`（退出 `2`，带字段）。

核心不认识任何服务商，也不知道 Hermes 的文件和接口；换一个实现了这条命令的 Agent 插件，核心和 App 都不用改（ADR-0015）。

### 3. App：一页原生的“模型”，浏览器在 App 里开

页面和状态在客户端核心（`AgentModels` 的 ViewModel 和动作），壳只画和调平台能力。页面的结构：

1. 顶上是在用的模型：服务商、模型名、最近一次测试的结果；没选时是一句“还没接模型”和下面的服务商列表。
2. 服务商列表：插件标了推荐的在前，其余收进“更多”，带搜索；已接好的带勾和账号名或尾号。
3. 点一个服务商进它的一页，按它的 `auth` 给按钮：
   - **登录**（`device_login`）：没有码或码已经拼在登录网址里时，点了直接在 App 里打开登录网址；码要人抄的，先在页面上
     大字显示码，点“复制并打开登录页”才打开，这时关掉浏览器回来看码不算取消，取消用页面上的按钮。客户端核心按 `interval` 轮询，`approved` 时自动关掉浏览器、回到这一页，接着进选模型。`denied`、`expired`、`failed`
     如实说，给“重新登录”。
   - **一键登录**（`browser_login`）：iOS 用 `ASWebAuthenticationSession`，Android 用 Custom Tabs 加回跳的 intent filter，
     回跳后壳把网址交回核心，核心调 `login_finish`；用户关掉授权页算取消，不报错。授权网址带 `state`：客户端核心记下发起时的值，
     回跳的地址不是 `wonton://model-login` 或 `state` 对不上时不交给插件，这次登录按失败作废；插件换凭据前再校验一次，
     一个会话只换一次（规范里写明）。Android 的 Auth Tab 要另接一套结果回调，等有服务商真走回跳再评估。
   - **填 key**（`api_key`）：一个密码框、一个“去拿 key”按钮（有 `key_url` 时）。按钮在 App 里打开网址，回到 App 后输入框旁边
     是粘贴按钮：iOS 用系统的 `PasteButton`，点它不弹读剪贴板的询问；Android 回到前台时读剪贴板，内容以 `key_prefix` 开头就直接
     提示“粘贴刚复制的 key”。iOS 不主动读剪贴板，主动读会弹询问。保存时插件先试 key：`rejected` 停在这一页说 key 不对，`unverified` 照存并说明“这家没法提前验证”。
4. 接好后自动进选模型：插件推荐的排第一并预选，点“用这个”就是 `select`。要确认贵模型时弹一句插件给的说明，确认后再存。
5. 选好后自动跑一次 `test`，把结果放在顶上；之后可以手动再测。测试会发一条很短的真实请求，可能计费，按钮下写明。

浏览器的选法：

| 场景 | iOS | Android |
| --- | --- | --- |
| 去拿 key、设备码登录（没有回跳） | `SFSafariViewController`，批准后 App 自己关 | Custom Tabs，批准后 App 把自己的页面拉回前台 |
| 有回跳的授权（`browser_login`） | `ASWebAuthenticationSession`，回跳地址用 App 的 `wonton://model-login` | Custom Tabs + intent filter |

不用 `ASWebAuthenticationSession` 打开没有回跳的网页：它会弹“要使用某网站登录”的系统询问，对“拿 key”是误导，而且等不到回跳只能
让用户自己点取消。

已接好的服务商可以断开（`disconnect`），断开当前在用的那家时顶上变回“还没接模型”，不偷偷换到别家。

### 4. 入口：引导、Bot 资料页、聊天里的提示

- **引导**：ADR-0034 的步骤改成 资料 → 你的助手（名字和头像，见第 5 节） → 接模型 → 选需求和应用 → 确认。“接模型”一步就是
  第 3 节的页面，不再打开任何插件网页，也没有“沿用当前模型”这个选项：已经接好的直接显示在顶上，点下一步就走。接模型和以前
  的网页一样当场生效，不进引导草稿；最后一页列出“模型：服务商 · 模型名”和测试结果，没接、没测、测试不通分开写。跳过照旧允许，
  跳过不算接好。
- **Bot 资料页**：每个 Bot 的资料页多一行“模型”，显示服务商和模型名，点进第 3 节的页面。按 ADR-0122，模型配置跟着 profile 走，
  每个 Bot 各是各的。
- **聊天里的提示**：Agent 插件在没模型、key 失效时回的那句话带一个打开这一页的 `wonton://` 链接（ADR-0041 加一种链接，接口 PR 定），
  不再指去插件网页的 Keys、Models 页。
- 插件网页照旧能从“应用”里打开，给要用 Hermes 控制台其他功能的人；它不再是模型设置的入口，引导和提示都不指向它。

### 5. 默认 Agent 叫云吞，第一次配置时能挑头像、改名字

- 默认 Agent 的初始名字是“云吞”（繁中“雲吞”，英文和其他语言 “Wonton”）。名字是 Matrix 资料的 `displayname`，只有一个值，
  建 Bot 时按主人的语言取；以后改名照 ADR-0044。名字写在 Hermes 插件清单的 `bot.display_name`，核心不写死。
- 初始头像是 ADR-0126 的主角色云朵：正面天蓝身体、单边白色螺旋独角、笑脸，由核心按 ADR-0126 合成，以 Bot 的身份写进 Matrix
  资料的 `avatar_url`，别的 Matrix 客户端看到的是同一张。
- 引导的“你的助手”一步：上面是头像，下面是名字（预填“云吞”）。头像两种来源，同 ADR-0126 的 Bot 资料页：云朵（取色器加六个
  预设色、表情、头饰，都来自 ADR-0126 的素材清单）和从相册选。名字和头像进引导草稿，点完成时经 ADR-0044、ADR-0126 的按 Bot
  资料接口一起写；合成和上传在核心，壳只挑。
- 按 ADR-0122，这个名字也是主 Agent 的名字：用户在这一步起名，就是在给主管起名。

### 6. Hermes 插件那一侧

都在 `wonton-plugin-hermes` 里，不改上游源码（ADR-0108 第 7 条）：

- 加 `wonton/models.py`，实现第 1 节的命令：把每个 `op` 翻成第“Hermes v0.23.4 已有的能力”表里的控制台接口，带上
  `profile=<bot>`；会话令牌经 `HERMES_DASHBOARD_SESSION_TOKEN=file:` 指向 init 生成在数据目录里的一个 0600 文件，控制台和命令读同一份。
  设备码会话因此留在控制台进程里，由上游自己的后台线程轮询、保存。
- 推荐哪几家、什么顺序、`key_prefix`，写在插件里，跟着 Hermes 的服务商目录走；上游加了服务商，`catalog` 自然带出来。
- 订阅登录只给上游控制台的设备码四家。`browser_login` 只给 OpenRouter，并且要先验证它接受 `wonton://` 回跳（见“待验证”），
  验证之前和不接受时 OpenRouter 只给填 key。
- `test`：用这个 profile 解析出来的主模型和凭据发一条极短的请求，只看通不通，按结果分类，不把上游原文当成功。
- 清单：`bot.display_name` 改成云吞，默认 Bot 的云朵由核心按 ADR-0126 给，插件不另带头像文件；`webui.path` 不再落在 `/models`；“关于”里的模型设置说明改成在 App 里设。
- 没模型时的提示（`wonton/texts.py` 和语言包）改成指向 App 里的模型页。

## 理由

- **入口在 App，能力在 Hermes**：主人要的是体验，不是另一套模型接入。原生页只做列、填、登、选、测，背后每一步都是 Hermes
  自己的接口，升级 Hermes 时服务商、登录方式跟着变，Wonton 不维护 provider 目录（ADR-0108 第 4 节）。
- **合同在插件，不在核心**：核心只认“Agent 插件有一条模型接入命令”，换 Agent 插件不改核心（ADR-0015）。和 ADR-0110 的 Bot
  命令、ADR-0114 的账号命令同一种形状，核心已有执行、超时和退出码的处理。
- **命令而不是转发 HTTP**：每个 Bot 在哪个容器、哪个 profile 由核心决定，exec 进去天然带上；不用给插件再开一个监听端口、
  再定一套鉴权。设备码要的长期状态留在 Hermes 控制台进程里，命令本身无状态。
- **key 不在核心落地**：存在 Bot 自己的数据里是 ADR-0108 定的，核心多存一份就多一处过期和泄露，也会和 Hermes 自己的 `/model`、
  Keys 页各说各的。
- **App 内浏览器按有没有回跳来选**：没有回跳的网页用 `SFSafariViewController`、Custom Tabs，App 靠轮询知道什么时候关；
  有回跳的才用认证会话，系统询问只在真正“用某网站登录”时出现。

## 代价与风险

- 插件要多写一个适配层，跟着 Hermes 控制台接口的变化改。控制台接口是上游给自家网页用的，没有版本承诺；每次升级 Hermes 都要跑
  这条命令的冒烟（列、填、登、选、测各一次）。
- 设备码登录要人在浏览器里登服务商的账号：`SFSafariViewController` 不共享 Safari 的登录状态，第一次要重新登；Custom Tabs
  共享 Chrome 的。
- 测试会花一点钱。没有探针的服务商，保存 key 时只能回“没法验证”，要等测试才知道。
- 订阅登录只有上游控制台支持的那几家；Claude 订阅、Qwen CLI 在 App 里登不了，不列出。
- 容器里的工具读得到 profile 的 `.env`，key 和登录态的这个风险照 ADR-0108 写明，本篇不改变。
- 一个 Bot 一页模型设置。同一个人的几个分身要用同一个 key，得各填一次（ADR-0110 新建时会从宿主 profile 复制一份）。

## 没选的做法

- **照旧打开 Hermes 控制台**：主人否了。
- **客户端核心直接调 Hermes 控制台的接口**：App 和核心都写死 Hermes，换 Agent 插件要改客户端，违反 ADR-0015；会话令牌也得交给手机。
- **用插件设置和输入密钥（ADR-0077、ADR-0064）声明 key 和模型**：只能是固定的表单，列不出上游动态的服务商和模型，做不了
  设备码登录；值存在核心、按插件一份，不能按 Bot；改一次重建容器；和 Hermes 自己改的 `.env`、`config.yaml` 是两份事实。
- **插件给页面描述、App 照着画（ADR-0120 的组件树由插件发）**：通用，但界面交给插件作者，登录、粘贴、回跳这些平台能力还是要单独
  定合同；模型设置是 Agent 都有的一件事，按领域定数据更好测、更好做体验。ADR-0120 落地后，这一页的普通行和字段可以改用它的组件。
- **经 MCP 工具和 elicitation**：App 不是 MCP 客户端；把改 key、换模型做成工具还会让 Agent 自己看得见、调得到。
- **核心替所有服务商做 OAuth**：就是 ADR-0108 否掉的“Wonton 自己的登录适配”，而且服务商的 OAuth 客户端是 Hermes 注册的。
- **起一个常驻的插件 HTTP 服务**：要另开端口、另定鉴权，状态也只是把控制台已有的再存一份。

## 待验证

验证都以 Hermes `v0.23.4` 的官方镜像为准，写明设备和系统版本：

1. `HERMES_DASHBOARD_SESSION_TOKEN=file:` 在官方镜像的 s6 启动方式下生效，命令能用同一个令牌调写接口。
2. `?profile=<bot>` 对 `/api/env`、`/api/providers/oauth`、`/api/model/*` 都只读写那个 profile。
3. 设备码登录四家在 iPhone 的 `SFSafariViewController` 和 Android 的 Custom Tabs 里走得通，码能自动带上，批准后 App 能自动关掉
   浏览器回到原页。
4. OpenRouter 的 PKCE 授权是否接受 `wonton://` 作 `callback_url`；不接受时是否接受 https 通用链接。
5. `test` 在 key 失效、额度用完、网络不通时分得出 `rejected`、`upstream`、`unreachable`。
6. 不装 Magpie 的干净安装，从引导一路接好模型，第一句话有真实回复（ADR-0108 的验收）。

## 主人已定（2026-11-27）

1. **服务商全列**：Hermes 支持的都列，插件标了推荐的放前面，其余收进“更多”，能搜索。推荐的几家由 Hermes 插件挑，以 Hermes
   目录里实际有的为准。
2. **key 只存在这个 Bot 自己的 Hermes 数据里**，核心不留副本。
3. **订阅登录**：上游控制台支持的四家都给（Nous Portal、ChatGPT/Codex 订阅、xAI Grok、MiniMax）；OpenRouter 验证能回跳后再加
   一键登录；Claude 订阅、Qwen CLI 不列。
4. **头像**：用 ADR-0126 的分层云朵，默认是正面天蓝身体、单边螺旋独角、笑脸；挑头像时给颜色、表情、头饰和“从相册选”。
   云的轮廓只用主人原图做出的身体模板，不让模型重画身体；头饰在头顶右侧。
5. **引导顺序**：资料 → 你的助手（名字、头像）→ 接模型 → 应用 → 确认，名字头像单独一步。
