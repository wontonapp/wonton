# ADR-0034 新用户引导和帮助文档：选 Agent、接模型、按需求推荐，帮助由核心的工具提供

- 状态：已接受（2026-11-04）；2026-11-13 按主人“模型只有一个入口”的决定修订（[ADR-0055](0055-model-gateway-optional.md)）。页面的外框和选项行见 [ADR-0038](0038-multi-step-cards.md)
- 取代：[ADR-0007](0007-agents-and-bots.md) “范围”里 ACP 桥的时间（和 M1 同时开工，M1 验收不等它）；ADR-0030 “什么时候填”里配对后单独弹资料页的做法（并入引导）

## 修订记录

- 2026-11-13：保留官方订阅登录；模型服务商、key 和自定义上游配置统一进入 magpie 原生网页，删除原生表单、命令转交接口和索引服务商白名单的设计。主人原话、模型来源规则和影响范围见 ADR-0055。

## 背景

主人要新用户的第一次使用非常友好：选一个默认的 Agent，按自己的需求装应用，学会 App 的基础用法。用户对 Wonton 和 App 有问题时直接问自己的 Agent，Agent 能教他；教的内容跟着项目更新，家里主机连不上外网时也能答。

调研时查到的几件事决定了这篇的形状：

- **能装的 Agent 比能选的少。** Hermes 自带 Matrix，已是插件。Codex、Claude Code、omp 要 ACP 桥（ADR-0007），桥没有实现。OpenClaw 有官方的 Matrix 插件 `@openclaw/matrix`，可以不经过桥。
- **订阅不能都经 magpie。** Claude Code 的条款（2026-11-04 读）不许第三方代用户转发订阅请求，也不许第三方收集或中转账号凭据；不妨碍用户在未修改的 Claude Code 程序里登录自己的账号，平台托管也算。
- **skill 没有一个所有 Agent 都读的目录。** 见下表。经 MCP 发 skill 的 SEP-2640 已定稿，MCP 官网的支持表里只有 ChatGPT、fast-agent、MCP Inspector 标了部分支持。
- **Agent 住在家里。** 手机连不上家时，任何 Agent 都答不了。“离线能答”保护的是家里主机上不了外网或连不上 GitHub 的情况。云端模型也断了时，要接了本地模型才答得了。
- **magpie 的 library 帮不上。** `magpie library` 把 instructions、MCP 和 skill 写进同一台机器上的 Agent，Wonton 的 Agent 各在自己的容器里。它的 `docs/subsystems/library.md` 是这张表的重要来源。

| Agent | 接入 | skill 目录 | 怎么核实的 |
| --- | --- | --- | --- |
| Hermes v0.22.5+ | 自带 Matrix | `$HERMES_HOME/skills`；不读 `~/.agents/skills`，项目级的要 `hermes skills trust` | 隔离的 HOME 里 `hermes skills list`，源码 `agent/skill_utils.py` |
| OpenClaw 2026.9.5 | 官方 Matrix 插件 | `~/.agents/skills`；改了状态目录时用 `skills.load.extraDirs` | `openclaw skills list --json`，来源是 `agents-skills-personal` |
| Codex 0.166 | 经桥，`codex-acp` | `~/.agents/skills` | `codex debug prompt-input` 里有这个 skill |
| omp 18.16 | 经桥，`omp acp` | `~/.agents/skills` | `omp skill list --json` |
| Claude Code 2.1.318 | 经桥，`claude-agent-acp` | `~/.claude/skills` | 官方文档，没有实测 |
| Gemini CLI、OpenCode、Kimi Code、Cline、Crush、Droid、Grok Build | 经桥 | `~/.agents/skills` | 只看了 magpie 的文档 |

## 决策

### 流程

配对之后依次是：你的资料（ADR-0030 的页面）、选 Agent、接模型、选需求和应用、确认，然后进入和这个 Agent 的聊天。

- 资料、Agent 和应用选择记在引导草稿里，最后一页列出全部选择和应用要的权限，点完成才一起执行（ADR-0038 的“新用户引导”）。模型网页的保存不属于这份草稿，见“接模型”。
- 每一步都能跳过，跳过不写入该步的引导草稿，也不撤销主人已经在模型网页保存的修改。跳过不伪装成完成：没接模型时如实说明，不能把进入过网页当成模型已可用（ADR-0008）。
- 引导不等下载。点完成后保存资料、默认 Agent 和引导完成记录，Agent 和应用在后台装，最后一页显示每一项的进展，可以直接进入聊天；Agent 还没好时，聊天列表如实显示“安装中”（ADR-0007）。
- 走没走完记在核心的 `core/onboarding.json`：第二台设备不重复，换一台服务器重新走。设置里能重看，重看时每一步显示现在的设置，已经配好的不跳过。
- 接口是 `GET`、`PUT /_wonton/api/onboarding`，鉴权同 `owner-profile`：主人令牌，或者经隧道的已配对设备，且不是别的网页发起的。Bot 不能调。
- 状态和判断都在客户端核心：`OnboardingViewModel` 给出当前步骤、列表和状态，壳只渲染，操作发回核心。行为测试覆盖步骤的顺序、跳过和各种失败。

### 选 Agent

一页列表，每行是纯文字的名字和接模型的方式，如“Codex：ChatGPT 订阅登录或模型网关”。默认选中第一项。

- **名单来自官方索引的 `onboarding.agents`**（见“推荐清单”），加上已经装着的 Agent，不用发 App。只列在真实主机上冒烟通过的 Agent 插件。第一批 Hermes、OpenClaw；ACP 桥做好后 Codex、Claude Code、omp；再后面 Kimi Code、Qwen Code、OpenCode（ADR-0079，不收录 Google 的命令行 Agent）。这里只列官方的，社区的 Agent 在商店里。
- **名字用文字，图标不用官方标志。** 标志由各家的商标规则约束，Claude Code 的条款只允许用纯文字说“运行 Claude Code”。新的 Agent 插件统一用首字母图标。
- **“默认 Agent”只决定聊天列表里排第一的是谁。** 核心记下 `primary_agent`，不按它路由，也不特殊处理（ADR-0015）。设置里能改。
- **Hermes 照旧预装（ADR-0007）。** 选了别的，最后一页列出它的权限，点完成后在后台装上；新 Agent 能聊之后问一次要不要卸掉 Hermes，能省约 4 GB。卸载走原有流程。

清单新增 `agent.login`，声明官方程序自己的账号登录。没写就是只能接模型网关：

```yaml
x-wonton:
  kind: agent
  agent:
    protocol: acp
    login:
      - id: chatgpt
        title: { zh: ChatGPT 账号, en: ChatGPT account }
```

声明 `login` 的插件要申请 `network.egress`（ADR-0007）。安装确认页如实列出。

### 接模型

模型来源遵循 [ADR-0055](0055-model-gateway-optional.md#agent-的模型来源只有两种)：官方订阅登录，或者 magpie 网关；不提供直接给 Agent 填 API key 或外部地址的第三种接法。

**订阅只在 Agent 官方程序里登录。** 选了声明 `login` 的 Agent 时，这一步保留“用 ChatGPT 账号”这类选项。主人明确选择后不要求先配网关，ACP 桥把官方的登录链接发进聊天（ADR-0007 的两条通用路径）。登录态放在这个 Bot 的数据目录里，模型请求直接发给官方，不经过 magpie；它不是缺网关时自动选中的后备。App 的引导和帮助文档不提“经 magpie 用订阅”；magpie 网页里的功能不改，也不推荐这种用法。

| 账号 | 做法 | 依据和限制 |
| --- | --- | --- |
| Claude | 未修改的 Claude Code，`claude-agent-acp` 启动，官方流程登录 | 条款允许用户在托管的未修改程序里登录。条款同时要求基于 Agent SDK 做产品的开发者用 API key，而这个适配器基于 Agent SDK，所以经桥算不算没有明说（条款读于 2026-11-04），风险在用户自己的账号。主人决定不等书面确认，公开版本同内测一样开着 |
| ChatGPT | Codex，设备码登录（`codex login --device-auth`） | 官方文档写的无头登录方式，自动化场景建议用 API key。使用条款原文和 Sign in with ChatGPT 的自托管说明没读到 |
| 其他 | Kimi Code 的设备码等，按各家条款逐个看 | 接入时核实 |

Claude Code 的程序不打进我们发布的镜像：插件在第一次启动时下载官方原版，不修改。

**模型设置只打开 magpie 原生网页。** 聊天侧栏的“模型设置”和引导里的网关配置入口打开同一个插件网页，不放进某个 Bot 的设置里。侧栏应用的加入与编辑沿用 [ADR-0095](0095-no-tab-bar.md)；未加入侧栏时仍可从全部应用进入。

- 填服务商、API key、自建地址和模型名都在网页里完成；切模型、测存活、看用量分别用 magpie 自带的 Providers、Routing、Usage，不另造原生表单或切换、测试页面。
- App 通过 [`ModelSettingsPage`](../../crates/wonton-protocol/src/onboarding.rs) 保留插件身份和 `web_url`，复用既有网页容器，不按 URL 猜插件身份，不替 magpie 调内部网页接口；核心不再为这套原生表单定义 `provides.models.providers` 的 list/save/test 命令、服务商白名单或转交 API。模型访问凭据契约仍归 ADR-0008。
- 网页按自己的保存动作生效，不等引导的“完成”。返回、跳过或关闭引导不回滚已保存的网页修改，最后确认也不重复保存 key；key 不进入引导草稿或聊天记录。
- 已接模型时可以沿用；要修改仍打开同一个网页，不在引导里复制服务商列表。网页打开过、网关能响应、模型列表非空，都不能单独证明上游模型存活；Test 可能发真实请求并计费，沿用 magpie 的说明。
- 网关未安装或未就绪时如实显示原因，不改连别处，也不改回原生填 key 页。主人可以重试或跳过，不能把跳过显示成模型可用。

### 按需求推荐应用

只问需求，可以多选：备份照片、听音乐、整理文档这些直接对应应用。选中的需求下面列出推荐的插件，点完成时装上，不用一个个点安装。

- 最后一页列出每个要装的应用的权限和说明，内容同安装确认页（安装预览），高权限醒目。点完成就是确认，然后在后台装，每个应用显示排队、安装中、装好或失败；失败的写明类别、可以重试，别的照常装。
- 要邮箱的插件在最后一页显示 ADR-0030 的 `missing_profile`，完成不能点：回资料页填邮箱，或者不选它。
- 已经装的、这台主机装不了的（`blocked`）不列或者如实标出，不藏原因。
- 官方索引取不到时，这一步说明原因，可以跳过。没有内置的后备清单：推荐的插件也要靠索引才能装，后备清单只能列出装不了的东西。
- Bot 发起的安装不变：一律走确认卡片，主人点了才装，高权限也一样（ADR-0033）。

### 推荐清单

放进官方索引 `wonton-store` 的 `index.json`，核心本来就按 ADR-0017 定时取它、核对、存盘，不用另一条路。`format` 不变：多出来的键老核心不读。

```json
{
  "format": 1,
  "plugins": [],
  "onboarding": {
    "agents": ["com.nousresearch.hermes"],
    "needs": [
      {
        "id": "photos",
        "title": { "zh": "备份照片", "en": "Back up photos" },
        "plugins": ["app.immich.immich"]
      }
    ],
    "prompts": [{ "zh": "你能帮我做什么", "en": "What can you help me with" }]
  }
}
```

- `agents`、`needs[].plugins` 只能写同一份索引里的插件，`agents` 引用的插件须是 Agent。`update.sh` 和 `plugin check-index` 检查引导字段和引用，写错了生成失败。所以 Hermes 也要进官方索引：主人卸掉之后，能从引导和商店装回来。
- 模型服务商与推荐由 magpie 原生网页管理，不在 `onboarding` 中定义 `providers`、`more_providers` 或 `key_url`，避免索引和网关各维护一套模型入口。
- 需求不带图标：图标是 App 打包的白名单（ADR-0026），不发 App 就加不了新的。

### 教用法：在用的时候教

不做滑动教学。

- 第一次进入和默认 Agent 的聊天时，输入框上方给几个可点的问题（`onboarding.prompts`）。点了放进输入框当草稿，用户点发送才发，和 ADR-0033 的 `ui/message` 同一个做法。Agent 查帮助文档回答，所以这同时在教“有问题就问它”。发过第一条消息后不再显示。
- 第一次出现确认卡片的提示条、第一次走到应用的完成页时，各提示一句。记在设备上，每台设备一次。
- 引导和提示的文字在客户端核心，推荐问题来自索引。

### 帮助文档

**文档只有一份，在 wonton-docs 的 `user/`。** 公开，人也能直接读。按语言分目录，每页是一个 Markdown 文件：

```text
user/
  zh/devices.md
  en/devices.md
```

```markdown
---
title: 加设备和换手机
summary: 再加一台手机、设备都丢了怎么办
requires_core: 0.9.0
---
```

- `title`、`summary` 必填，用来生成目录。`requires_core`、`requires_app` 只在这一页讲的功能比索引的 `docs.min_core` 新时才写；一页里只有一部分要新版本时，在那一段里写“需要核心 ≥ x”。
- 文件名是主题，公开接口，不随便改。写中文和英文，其他语言由 Agent 翻译。
- 不写真实地址、主机名和内部渠道名。

**核心提供工具 `wonton__help`，不敏感，有 `tools` 权限的插件都能调。**

| 参数 | 说明 |
| --- | --- |
| 无 | 目录：每页的主题、标题和一句话说明 |
| `topic` | 那一页的全文。不认识的主题返回 `invalid_input`，并列出可选的 |
| `lang` | 可选，默认用主人的语言，缺这种语言时回退规则同插件文字 |

每次都带上 `core_version`、`docs_commit` 和文档的时间；取新版失败时再加原因，如“连不上 GitHub，这是 3 天前的文档”。

**skill 是一张指路条。** `skills/wonton-help/SKILL.md` 在 wonton-docs 里，十行，只写什么时候用和调这个工具，不带文档内容。Agent 插件把它放进自己的 skill 目录（Hermes 由 init 服务拷进 `/opt/data/skills`，别的按上表）。官方 Agent 插件都带；第三方的不带也能调工具。

**文档只缓存在核心。**

- 核心镜像构建时打进一份（`/usr/share/wonton/docs`），和 ADR-0033 的 `wonton-plugin` 说明放在一起，所以新装好的主机一定有文档。`WONTON_DOCS` 能换目录，开发和测试用。
- 官方索引多一个 `docs`，核心取索引时看它：

  ```json
  {
    "docs": {
      "source": "github:wontonapp/wonton-docs",
      "commit": "<40 位提交号>",
      "min_core": "0.9.0",
      "files": { "user/zh/devices.md": "<sha256 的十六进制>" }
    }
  }
  ```

- 提交变了就取变了的文件，取法和核对同插件包（ADR-0017），代理同 ADR-0023。全部核对通过后才切换到 `plugins/store/docs/<提交号>/`，半途失败时旧的照常用。
- 总用最新的。这台核心比 `min_core` 旧时不切换，停在它能用的最后一版，并在工具结果里说明要升级核心。核心的版本号和升级接口报的是同一个（ADR-0025，镜像的 `org.opencontainers.image.version`），不是 Cargo 里的版本。开发构建没有版本号时不比较，`core_version` 如实写未知。
- 文档合并到 wonton-docs 不会马上到用户手里，要在 wonton-store 里更新 `docs`，所以发布有一步人工把关。

**手机连不上家时，** App 显示连接诊断（ADR-0032）和公开文档的链接。App 不存文档。

**没接模型时，** Agent 答不了问题，引导和提示会先让用户去接模型（ADR-0008）。

**和 ADR-0033 的关系。** `wonton__plugin_guide` 给的是这台核心的规范版本对应的说明，所以只用镜像里那份，不跟随索引更新。帮助文档要跟随，二者不合并成一个工具。

## 理由

- **工具是唯一的来源：** 所有连着 MCP 总线的 Agent 都能用，不依赖 skill 目录和 SEP-2640 的支持情况。skill 负责让 Agent 知道该调它。
- **缓存在核心：** 换 Agent 不用重做，也不会几个 Agent 说的不一样。核心已经在取官方索引，多取几个文件不是新机制。
- **文档总用最新：** 改错字和讲 App 先发的功能不用等核心发版。标最低版本后，不会把用户带去找没有的按钮。
- **推荐清单放进索引：** 不发 App 就能改，也和插件本身一起更新，不会推荐到索引里没有的东西。
- **引导不依赖 Agent 对话：** 模型接上之前 Agent 说不了话；资料和应用选择用原生页面，模型配置复用网关网页。
- **教用法放进聊天：** 滑动教学看过就忘，文字也容易过时。让用户问一次，学到的是以后真正要用的办法。

## 没选的方案

- **ACP 桥进 M1 验收：** 内测时就有主流 Agent，但 M1 要晚一段。
- **M1 只有 Hermes，不做选择页：** 和主人要的相反。
- **不预装 Hermes，选完再下载：** 配对后要等约 5 分钟，ADR-0007 已经否过。
- **订阅都经 magpie：** 一处配齐，Hermes 也能用订阅。Claude 的条款明确不许，用户的账号有风险。
- **只支持填 key：** 只有订阅的人要多花一份钱。
- **原生表单与网关网页并存：** 两处都能配模型，主人难以知道在哪里切换、测试；统一复用模型设置网页。
- **选择页先问“你有哪个 AI 账号”：** 多一步，也更难看到全部 Agent。列表每行写接法，有 ChatGPT 的人一样看得到 Codex。
- **问职业：** 职业到应用隔了一层，要多维护一张表，猜错的时候也更多。
- **推荐清单写在 App 里：** 改一次要发一次版。
- **整个引导都让 Agent 带：** 模型接上之前做不到。接好之后可以再加。
- **skill 带全部文档：** 每个插件要多挂载一个目录、解决更新，不读 skill 的 Agent 用不上。
- **只有工具：** Agent 的 skill 列表里看不到它，全靠工具描述触发；主人要默认带 skill。
- **只用 SEP-2640 发 skill：** 主流 Agent 还不支持。支持多起来后，核心可以把同一个目录也这样发出去。
- **每个 Agent 插件各存一份文档：** 换 Agent 就要重做，版本还可能不一致。
- **App 也存一份：** 要多做阅读页和一条更新路径。手机连不上家时公开文档的链接够用，要是不够再加。
- **文档跟核心版本走：** 说的和机器一定对得上，但改错字要等发版，App 先发的功能也查不到。
- **帮助文档做成系统插件：** 多一个容器，只为读几个文件，安装好之前还答不了。
- **核心调用 magpie 内部网页接口来配模型：** 核心会耦合上游内部实现。直接打开原生网页不需要这条接口。

## 代价与风险

- **Claude Code 的条款。** 条款读于 2026-11-04，没说经 `claude-agent-acp` 用订阅登录算不算允许的情形，也没说桥把用户贴回的授权码转给 `claude auth login` 算不算中转凭据（ADR-0007 的终端类登录就是这样做的），风险在用户自己的账号。主人决定不等 Anthropic 书面确认，公开版本和内测一样直接开着订阅登录，不是只提供 API key。这里读的是条款页面，不是法律意见。
- **ChatGPT 的条款。** 只读到官方文档，使用条款原文没读到。
- **桥是新的大块工作。** 它要做每个 Bot 的加密、确认、登录和排队（ADR-0007）。名单从索引读，所以桥没做好时只是选择页上少几个 Agent。
- **OpenClaw 没有实测。** Matrix 插件的加密要原生绑定，文档说支持，容器里没跑过。加密跑不通就不上架，不退回明文。
- **omp 只验证到握手。** 会话、确认和接 magpie 要在桥上实测。
- **模型设置网页的复杂度。** Providers、Routing、Usage 保留上游原生体验；如果能力不足，应记录缺口，不再另做一套配置、测试或用量界面。
- **跳过的人可能一直没有模型。** 靠 Agent 私聊和“模型设置”的提示兜着，不反复打扰。
- **文档可能和机器对不上。** `requires_core` 靠写文档的人标，漏标时 Agent 会教用户找不到的按钮。工具结果里有版本，Agent 和用户能查出来。
- **文档也是给模型读的内容。** 来自我们自己的公开仓库，按提交和 sha256 固定，有人工把关，仍按 ADR-0011 当成工具结果处理，不给它额外的权限。
- **公开接口变多：** `user/` 的文件名、`onboarding` 和 `docs` 的格式、`agent.login` 都是契约；模型设置页不再增加原生服务商表单的契约。

## 已定

主人 2026-11-04 定了，都按推荐：

1. **Agent 范围：** 选择页先做，名单从商店读，第一批 Hermes、OpenClaw。ACP 桥马上开工，做好后上 Codex、Claude Code、omp。M1 验收不等桥。
2. **Hermes：** 照旧预装，选了别的再多装一个，问一次要不要卸。
3. **订阅：** 填 key 走 magpie，订阅只在 Agent 官方程序里登录。经桥用 Claude 订阅，公开版本和内测一样直接开着，不等 Anthropic 书面确认（主人 2026-11-05 改）。
4. **模型设置：** 按 2026-11-13 的修订进入 magpie 原生网页，入口与保存边界见“接模型”；不另做填 key、切模型和测试页。
5. **选择页：** 一页列表，每行写用什么账号接，名字用文字。
6. **推荐：** 只问需求。
7. **推荐清单：** 放进 wonton-store，核心和官方索引一起取。主人同意时清单写的是“镜像带一份底”，写这篇时发现推荐的插件也要靠索引才能装，后备清单没有用处，所以去掉了，已告知协调会话。
8. **教用法：** 在用的时候教。
9. **帮助：** 工具是唯一的来源，skill 是指路条。
10. **缓存：** 只在核心。
11. **版本：** 总用最新，新功能标最低版本。

## 验证

2026-11-04、2026-11-06 的记录对应各段所列版本，包含修订前的原生模型表单；它们不是 2026-11-13 网页入口决定的端到端验收。

### Agent 会不会调帮助工具（2026-11-04）

假的 `wonton__help`（本机 stdio MCP 服务，只有这一个工具，内容里有一个文档之外猜不到的词）加上真实的 Agent 和模型（GLM-5.3-Flashx，经开发机的 magpie）。Hermes v0.22.5+1711、Codex 0.166、omp 18.16，都在隔离的 HOME 里，不在容器里。问题是“我想再加一台手机，Wonton 怎么加设备？”，每种情况跑两次。

| | 带 skill | 不带 skill |
| --- | --- | --- |
| Hermes | 2 次都调了，都用了文档 | 2 次都调了，都用了文档 |
| Codex | 2 次都调了，都用了文档 | 2 次都调了，都用了文档 |
| omp | 2 次都调了，都用了文档 | 2 次都调了，都用了文档 |

- 工具本身够 Agent 找到，这次没有测出 skill 带来的差别。问题里点了 Wonton，也只有这一个工具，是最容易的情形。问题不提 Wonton、工具很多时有没有差别，没有测；Claude Code 和 OpenClaw 也没有测。
- 没带 `topic` 先看目录的和直接猜主题的都有（`add device`、`加设备`）。假的服务收什么都给那一页；真的工具要在 `invalid_input` 里列出可选的主题，让 Agent 接着选。

### 实现时要验证

- 真实主机上从配对走完引导：选 Hermes、打开 magpie 网页配置真实上游、返回引导装一个推荐应用、点推荐问题、Agent 答对。跳过每一步各走一遍；关闭引导不回滚网页已保存的修改。
- 取不到索引时，引导如实说明，`wonton__help` 用镜像里的文档回答。
- 取到新文档后 `docs_commit` 变化；半途断网时旧文档照常用；`min_core` 比核心新时不切换并说明。
- OpenClaw：容器里用 Matrix 加密私聊收发，经 magpie 调模型，能调总线工具。
- omp：经桥走完会话、确认、打断和续聊，指向 magpie。
- Codex 和 Claude Code：经桥调总线工具（ADR-0007 没测 ACP 的 `mcpServers`），用账号完整登录一次，容器重启后登录态还在。
- 模型设置：从引导和侧栏进入同一个网页，能配置上游、改默认路由、测存活、看用量；错的 key、上游断网、网关未就绪如实报原因，不出现隐藏直连。

### 引导和帮助的核心层冒烟（2026-11-06）

`deploy/dev/core/smoke-onboarding.sh` 在开发 Docker 主机跑通。条件：核心取自 main（构建时的提交号 `4d70e68`），镜像的
版本号按官方索引的 `min_core` 写（`0.1.0-beta.1`，比较才有意义）；官方索引取自 wonton-store 的 main，文档 pin
`0e732eea`，16 页 blob 全部按 sha256 核对后放在本地；索引和 blob 在场景里原地替换，不需要外网。

- 取不到索引：`/onboarding` 报 `catalog_problem`、agents/needs/prompts 全空，不造后备清单；`wonton__help` 用镜像内置
  文档回答并带 `refresh_problem`。
- 文档更新：索引换一个内容可控的 `docs.commit`（devices 页加冒烟标记）后，`docs_commit` 和正文都变。
- 取新文档失败（本地索引按 sha256 取文件，指向一个不存在的 blob，和断网取不到文件是同一条失败路径）：旧文档
  照常服务，`refresh_problem` 写明原因，`docs_commit` 不变。
- `min_core` 99.0.0 比核心（0.1.0-beta.1）新：不切换，工具结果说明要升级核心。
- 模型网关三种错误的核心返回各不相同：错的 key 是 400 `invalid_input`（「拒绝了 key，key 没有保存」），断网是
  502 `upstream_unavailable`（「连接失败，key 已保存」），网关没启动是 503 `models_unavailable`。key 用
  `not-a-real-key` 字面量，没有登录任何服务、没有真实模型。

### 两端走查（2026-11-06）

核心取自 main 的 `93bc9a38`，官方索引和文档 blob 放在本地（文档 pin `0e732eea`）。设备是 iOS 26.5 模拟器和 Android 15 模拟器，从第一次配对开始。

- 配对、填资料、选 Hermes，装推荐的 Memos（确认权限后回到引导，显示已安装）：两端都通过。
- 从 App 内打开 Memos 网页，留在 App 里完成 OIDC 登录，身份是 owner：两端都通过。
- 完成后进入 Hermes 私聊。点推荐问题只填草稿，手动发送后真实模型回答；问怎么再加一台手机，Hermes 日志里调了 `wonton__help`，回答和 `user/zh/devices.md` 对得上：两端都通过。
- 重看引导后一路跳过，回到主界面：两端都通过。已接模型时模型这一步被跳过，符合上面的规则。
- 错的 key 被拒且不保存，网关停机、核心断连各显示不同的错误并能重试或跳过：iOS 实测；Android 实测了错的 key。

条件和没测的：
- 模型经开发机上的 magpie 接入（`magpie provider add upstream url=… key=placeholder`），没有在 App 里填真实厂商的 key，所以“填真实的 key”这条只验证了错的 key。
- `magpie provider test` 对可用上游（magpie 0.1.1327）输出 `✓ chat 345 ms · <模型名>`，退出码 0，输出不含 key，走 `providers.sh` 的成功路径。这个上游不校验 key，错的 key 同样通过，所以它不能代表真实厂商对 key 的校验；厂商拒绝错 key 的输出见 `providers.sh` 的注释。

## 参考

- [Agent Skills 规范](https://agentskills.io/specification)、[Skills over MCP](https://modelcontextprotocol.io/extensions/skills/overview)、[MCP 扩展支持表](https://modelcontextprotocol.io/extensions/client-matrix)
- [Claude Code 的 Legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)
- [Codex 的登录](https://learn.chatgpt.com/docs/auth.md)、[Codex 的 skills](https://developers.openai.com/codex/skills.md)、[Sign in with ChatGPT](https://developers.openai.com/siwc/quickstart)
- [OpenClaw 的 Matrix](https://docs.openclaw.ai/channels/matrix)、[OpenClaw 的 skills](https://docs.openclaw.ai/tools/skills)
- [magpie 的 library](https://github.com/yetone/magpie/blob/main/docs/subsystems/library.md)

## 关联

ADR-0007（Agent 与 Bot、ACP 桥）、ADR-0008（模型网关）、ADR-0011（安全模型）、ADR-0015（小组件）、ADR-0016（确认）、ADR-0017（官方索引）、ADR-0023（代理）、ADR-0026（图标）、ADR-0030（你的资料）、ADR-0032（连接状态）、ADR-0033（底栏和 Agent 写插件）。
