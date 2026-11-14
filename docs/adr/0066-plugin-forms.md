# ADR-0066 插件的几种形态：一条安装管线，按 Bot 生效，目录只给数据

- 状态：已接受（2026-11-08，主人定）

## 背景

插件只有一种形态：一份 compose 加 `x-wonton`，核心起容器。主人批了商店加“工具”和“技能”两区（ADR-0064 的来由），
可是这两类东西多数不是容器：
- **远程 MCP 服务器**：只有一个 URL，多数要 API key 或 OAuth（Linear、Notion、GitHub 的官方 MCP）。
- **技能（Agent Skills）**：一个目录，里面是 `SKILL.md`，可能带脚本，交给 Agent 读。
- **本地 MCP 服务器**：大多是 npm、PyPI 包，用 stdio，可以照现有规范包成容器插件，但每个都要作者手写一份 compose。

还有两件规范里早就说了要做、一直没做的事：
- **工具按 Bot 放行。** ADR-0006 要求总线按 Bot 的白名单放行，ADR-0016 写明等一个插件多个 Bot 时再加
  （ADR-0045 的 #578 正在做按 Bot 认调用方）。现在每个 Bot 都看得到所有插件的工具。
- **技能没有地方装。** ADR-0034 只解决了 `wonton-help` 一张：由 Agent 插件自己拷进它的 skill 目录。

调研（2026-11-08，插件底座）看了：
- 装技能和 MCP 的工具：vercel-labs/skills、Context7 `ctx7`、add-mcp、install-mcp、Smithery CLI；
- 带作用域的插件系统：Claude Code、Codex；
- 技能商店：Hermes skills hub；
- 编辑器：Cursor、VS Code、Goose；
- 其他：dsh/Cordis、Koishi、HACS、Umbrel、Start9、Docker MCP Catalog。

结论：
- 范围大多分两到三层：项目、用户，Claude Code 另有 local，Codex 另有 admin、system。Wonton 没有“项目”，对应的是 Bot。
- 指定装给哪个 Agent，靠硬编码的路径表：vercel-skills 的表有 80 多项。
- 只有 Hermes 在升级时保护本地改动。
- 没有一个工具记录是人装的还是 Agent 装的。
- 密钥大多明文写进 Agent 的配置。

## 决策

### 1. 运行形态：独立的 `runtime` 字段，`kind` 不变

`x-wonton.runtime` 说核心要运行什么；`kind`（app、agent、system 等）继续当业务标签：

| `runtime` | 核心做什么 | 包里有什么 |
|---|---|---|
| `compose`（有 `services` 时的默认值） | 起容器，同现在 | compose 加 `x-wonton` |
| `remote_mcp` | 不起容器，把远程服务器的工具接上总线（第 3 节） | 清单写 `url` 和鉴权方式 |
| `skills` | 不运行任何东西，只把技能交给 Bot（第 4 节） | `skills/<名字>/SKILL.md` |

- 任何形态的包都可以带 `skills/`。例如一个 MCP 插件带一张告诉 Agent 何时用它的技能，和 `wonton-help` 的做法一样。
- 静态网页（`static_web`）以后需要再加，这次不做。
- 规范版本 0.10（0.9 是一个插件多个 Bot）。不认识的 `runtime`，安装时说“先升级核心”（同 spec_floor）；App 收到不认识的取值落到 `Other`，不显示。

### 2. 包格式：非容器形态用 Agent Plugins 1.0

`remote_mcp` 和 `skills` 两种形态直接用 Agent Plugins 1.0 的包：`skills/` 加 `mcp.json`。Wonton 自己的字段放在规范的
`extensions` 里，键是 Wonton 的反向域名，内容和 `x-wonton` 一样（权限、密钥、`runtime`）。容器插件继续用 compose。

- 别家的包（Codex、Claude Code 的插件）只要是 Agent Plugins 1.0，就能从“添加的仓库”装进来（ADR-0031），
  走同样的预览和确认。
- 包里 Wonton 不支持的部分照规范“报告并忽略”。例如 `extensions` 里 OpenAI 的 apps、hooks，预览页如实写一行
  “这部分在 Wonton 里不生效”，不让主人以为装上了完整功能。

### 3. 远程 MCP：一个可替换的官方系统插件来连

核心不直接对外连远程 MCP，交给一个官方系统插件“远程 MCP 连接器”。它提供能力 `remote_mcp`（ADR-0053 的能力表），
一台服务器同时只有一个提供方，可以换成别的实现（同 ADR-0039 的代理插件）。

- **它做什么：** 按各个 `remote_mcp` 插件的清单外连、带上密钥、刷新令牌，再把工具以 Streamable HTTP 交给核心。
  在总线上，工具仍然挂在各自插件的名下（`<slug>__<工具名>`），确认、放宽、按 Bot 放行都按插件算，和容器插件没有区别。
- **用现成的。** 候选都是 Apache-2.0 或 MIT，作为独立容器运行。按文档逐个对过 OAuth 回调（2026-11-08，下面的“鉴权”要求连接器能接收从外面交进来的授权码）：
  - ToolHive（`thv run --remote-auth`）：回调固定在它自己机器的 `localhost:<端口>`，默认 8666。手机上的浏览器到不了那里；改用 `--remote-auth-bearer-token-file` 又没有刷新。按原样不合用。
  - IBM mcp-context-forge（7d34038）：`redirect_uri` 可以配，令牌加密存进数据库，也做 SSRF 检查。但它不合用：
    - 没有 CIMD，只有动态注册；
    - 回调接口回的是 HTML 页面，发起授权又要先有它自己的登录态；
    - 镜像 579 MB，实测空闲占 1.3 GiB 内存。
  - mcp-proxy、supergateway、mcp-remote：没有 OAuth，或者回调也在本机 localhost。

  **选：用官方 Rust SDK rmcp（v3.9.0，Apache-2.0）的 `auth` 写一个薄连接器。**
  - 它的 API 本来就是“外面拿到 code 和 state 再交进来”（`handle_callback`），PKCE verifier 留在连接器里。
  - CIMD 优先，动态注册兜底；令牌过期自动刷新；OAuth 请求读代理变量，并过滤内网地址。
  - 示例构建 18 MB，175 个依赖，没有 GPL。
  - 要自己写的：令牌和授权状态的持久化（库默认只放内存）、MCP 连接前的地址检查、把 `list_changed` 转给核心。估计 700–1000 行。

  连接器对核心的接口只有“code 和 state 交进来、工具出去”，以后要换成 forge 这类现成网关，核心不用改。
  Docker 的 mcp-gateway 要 Docker socket，不选。
- **出网：** 经出口容器或宿主代理（ADR-0023、ADR-0039）。没有代理又直连不上时，报“传输不可用”，不报“上游出错”。
- **SSRF：** 连接器只连解析后是公网地址的主机，每次连接都重新检查。要连局域网地址的插件，必须申请 `lan` 权限（ADR-0019）。
  回环地址、核心的端口和 Docker 网段一律拒绝。
- **确认页：** 写明主机名，加一行“你在这里的数据会发到 <主机>”，风险级别同出网。
- **鉴权**有三种，清单里选一种：
  - 不要鉴权。
  - 请求头带密钥：密钥用 ADR-0064 的 `input`，Agent 看不到。
  - OAuth，见下一节。

### 3a. 远程 MCP 的 OAuth：App 拿授权码，连接器换令牌

**卡在哪：** 回调地址。MCP 授权规范（2025-11-25 版“Communication Security”）要求 redirect URI 只能是 `localhost` 或 HTTPS。

- RFC 8252 里原生 App 用的自定义 scheme 不合规范。
- 连接器容器的 localhost，手机上的浏览器到不了。
- 家里的服务器没有公网 HTTPS（ADR-0021 排在以后）。

**做法：** 回调落在一个公开的静态 HTTPS 页面上，App 接住授权码，经隧道交给连接器。

1. 主人在 App 里点“连接 Notion”。连接器生成 PKCE 和 `state`，把授权地址交给 App。
2. App 用系统的认证会话打开授权页：iOS 的 `ASWebAuthenticationSession`，Android 的 Custom Tabs。主人在服务商那里登录、同意。
3. 服务商跳到 `redirect_uri`：公开文档仓库 wonton-docs 的项目 Pages 上的一个静态页面
   `https://wontonapp.github.io/wonton-docs/oauth/callback/`。页面用脚本跳到 `wonton://oauth/callback?code=…&state=…`，
   App 接住（iOS 的认证会话按 `callbackURLScheme` 收，Android 按 intent filter 收）。页面上同时有一个“打开 Wonton”的链接，
   脚本没跑时主人点一下也能回去。
   - 不用 universal link、App Links：那两个文件要放在域名根目录的 `/.well-known/` 下，项目 Pages 的根目录不归这个仓库。
     以后换成自定义域名时再加，现在的跳转照样能用。
4. App 把 `code` 和 `state` 经隧道交给核心，核心转给连接器。连接器用自己留着的 PKCE verifier 换令牌。令牌和刷新只在连接器里，不进 App，也不进 Agent。

回调页和 CIMD 文档（`oauth/callback/index.html`、`oauth/client.json`）在 wonton-docs 里，和这篇一起合。

公开页面上什么都不存，也不带主人的任何地址，页面和 `client.json` 里只有 Pages 的公开地址和 `wonton://`：
- 它只把查询参数转给 App，所有人用同一个页面。
- 授权码会出现在 GitHub Pages 的访问日志里，但没有 PKCE verifier 换不了令牌，而 verifier 只在连接器里。
- 不经过公开页面的路都走不通：localhost 到不了，自定义 scheme 不合规范，WebView 里登录又会被 Google 等账号拒绝。

**客户端注册**：按 MCP 规范的顺序，先 CIMD，再动态注册（DCR），最后预先注册。2026-11-08 查了三家官方 MCP 服务器的元数据：

| 服务器 | CIMD | 动态注册 | 怎么接 |
|---|---|---|---|
| Notion（`mcp.notion.com`） | 支持 | 支持 | CIMD：`client_id` 是 `https://wontonapp.github.io/wonton-docs/oauth/client.json`，写着回调页面的地址 |
| Linear（`mcp.linear.app`） | 支持 | 支持 | 同上 |
| GitHub（`api.githubcopilot.com/mcp`，授权服务器 `github.com/login/oauth`） | 不支持 | 不支持 | 只能预先注册。支持设备码流程，不需要回调，但要我们注册一个 GitHub OAuth App。第一版先用个人访问令牌（`input` 密钥） |

有这一份静态文档，CIMD 就不用每台服务器各注册一次；只支持动态注册的服务器，由连接器在第一次连接时注册，回调地址同上。

**没验证的：**
- Notion、Linear 的授权服务器接不接受这个回调页面：开 Pages 后用脚本带 PKCE 打它们的 `/authorize`，只看回应、不登录。
- 各平台浏览器从 HTTPS 页面跳 `wonton://` 时会不会多问一次“要打开 Wonton 吗”。
- GitHub OAuth App 的设备码令牌能不能调 `api.githubcopilot.com/mcp`。

- **工具清单漂移：** 远程服务器可以不升版本就加工具、改描述。照 ADR-0024 处理，变了也照样放行（主人 2026-11-08 定）：
  - 插件的“始终允许”和 Bot 的“一律允许”对新增的、描述变了的工具同样生效，不要求重新确认；
  - 总线照常转发上游的 `list_changed`，Bot 下一次 `tools/list` 就看得到新工具；
  - 详情页的工具列表现取，主人想看随时看得到。

### 4. 技能：核心存放，挂进各家 Agent 原生读的目录

主人定的方向（2026-11-08）：技能按各家 Agent 原生读取的目录挂进容器，Agent 自己会读，核心不替它转发。

**调研结论**（2026-11-08，源码按表里的发布版本核对：Codex rust-v0.167.0、Hermes v0.23.0、OpenClaw v2026.11.1、oh-my-pi v18.18.1；Claude Code 按官方文档 code.claude.com/docs/en/skills）：

| Agent（核对的版本） | 读 `~/.agents/skills` 吗 | 自己写技能到哪 | 扫几层 | 改了什么时候生效 |
|---|---|---|---|---|
| Claude Code 2.1.311 | 不读（只读 `~/.claude/skills`、项目 `.claude/skills`、企业目录 `/etc/claude-code/.claude/skills`、插件目录） | `~/.claude/skills` | 文档只写一层 | 监视个人和项目目录，中途生效；企业目录未核实 |
| Codex rust-v0.167.0 | 读（还读 `$CODEX_HOME/skills`、管理员目录 `/etc/codex/skills`） | `$CODEX_HOME/skills` | 递归 6 层 | 文件监视，约 10 秒 |
| Hermes v0.23.0 | 默认不读；`skills.external_dirs` 配了才读 | `~/.hermes/skills` | 递归 | 按会话缓存，下一次对话 |
| OpenClaw 2026.11.1 | 读（`skills.load.extraDirs` 也读） | `<ws>/skills`、`~/.openclaw/skills` | 递归 6 层 | 约每分钟扫一次，下一轮 |
| omp 18.18.1 | 读（默认开） | `~/.omp/agent/managed-skills` | **只扫一层** | 会话开始时 |

- **没有统一的目录。** Agent Skills 规范（已捐给 Linux 基金会 AAIF）只定义技能目录里放什么；`~/.agents/skills` 是规范的实现指南里写的通行约定，不是强制。Codex、OpenClaw、omp 原生读它，Hermes 要配置，Claude Code 不读。
- **不能挂在 Agent 自己写技能的目录上。** 只读挂过去会挡住 Agent 自己写、自己装的技能（Hermes 的 `skill_manage`、Codex 的 skill-creator 都往自己的目录写）。
- `~/.agents/skills` 也有人写：容器里跑 `npx skills add -g`（vercel-labs/skills）会写进去。只读挂上后，这条安装路径会失败。

**做法：**
- **存放：** 技能文件解包到核心的数据目录，每个版本一个目录，升级时整目录换掉，不在原地改。
- **每个 Bot 一个目录：** 核心为每个 Bot 维护一个目录，里面平铺这个 Bot 能用的技能（`<名字>/SKILL.md`，从存放处硬链接过来，不用符号链接：容器里解析不到宿主路径）。按范围（第 5 节）决定放哪些：全局的进每个 Bot，指定的只进那几个 Bot。平铺是因为 omp 只扫一层。
- **挂到哪里由 Agent 插件声明：** 清单写 `x-wonton.agent.skills.mount: <容器里的路径>`，核心建这个 Bot 的容器时把它的目录只读挂到那里。路径是各家 Agent 原生读、但不往里写的那一个。这样核心里没有各家 Agent 的路径表（ADR-0015），换一个 Agent 只改它自己的清单。官方 Agent 插件按调研这样写：

  | Agent 插件 | `skills.mount` | 另外要做的 |
  |---|---|---|
  | Claude Code | `/etc/claude-code/.claude/skills`（企业目录） | 无。企业层优先级最高，和 Bot 自己写的同名技能撞上时以我们的为准，见第 6 节 |
  | Codex | `/etc/codex/skills`（管理员目录） | 无。内置工具从不往这里写，排序最后 |
  | Hermes | `/opt/wonton/skills` | 插件的 init 把它写进 `skills.external_dirs`，优先级最低 |
  | OpenClaw | `/opt/wonton/skills` | 插件的 init 把它写进 `skills.load.extraDirs`，优先级最低 |
  | omp | `~/.agents/skills` | 无。原生目录优先级更高，Bot 自己写的在 `managed-skills`，不受影响 |

  没声明 `skills.mount` 的 Agent 插件，这个 Bot 不收技能，详情页写“这个 Agent 不支持装技能”，不假装装上了。
- **不重建容器：** 启用、停用、换范围只改这个 Bot 目录里的内容。Hermes 这类重建会丢令牌的 Bot（ADR-0023）不受影响。
- **什么时候生效，照实写：** Codex 约 10 秒，OpenClaw 下一轮，Hermes 和 omp 下一次对话；Claude Code 的企业目录要实测。详情页按这个写“下一次对话生效”之类，不假装已经生效。每个官方 Agent 插件都要实测，结果写进插件页。
- **带脚本的技能：** 确认页醒目写“这个技能带可执行的脚本，Bot 可能会运行它”。来源未审核的照常标“未审核”。
- **上限（补进 ADR-0037）：** 一个技能最多 512 个文件、16 MiB，同 SEP-2640 的建议。
- **以后：** Agent 普遍支持 SEP-2640 后，再加一条经总线以 `skill://` 发技能的路，挂载那条留着。

### 5. 装给哪些 Bot

“装”和“给哪些 Bot”分开：
- 每个插件在服务器上只装一份：容器、远程连接和技能文件都只有一份。
- 给哪些 Bot 只决定工具和技能对哪些 Bot 可见。这就是 ADR-0006 说的按 Bot 白名单：过滤 `tools/list`，拦截 `tools/call`。

**范围两种**（主人 2026-11-08 定），存在核心里，不写进插件：

| 范围 | 含义 | 以后新建的 Bot |
|---|---|---|
| 全局 | 这台服务器上所有 Bot | 自动得到 |
| 指定的 Bot | 勾选的几个 | 不得到 |

技能和 MCP 工具用同一套范围：
- **MCP 工具**落在总线的按 Bot 白名单上（#578 已经按总线令牌认出是哪个 Bot）：不在范围里的 Bot，`tools/list` 看不到，`tools/call` 回 403 `permission_denied`。
- **技能**按第 4 节挂进各 Bot 的技能目录：全局的进每个 Bot，指定的只进那几个 Bot。

每个 Bot 还可以对某个插件单独关掉，单独关掉压过范围。Bot 被删时，它的单独设置一起删。

**默认值看从哪里装：**
- 主人在商店装：确认页上选“全局”或勾选 Bot，默认“全局”。只有一个 Bot 时不显示这一项。
- Bot 经 MCP 装（ADR-0033 的卡片）：只给发起的这个 Bot，卡片上不改范围。要给别的 Bot，主人装好后在详情页改。
- 家人的 Bot（ADR-0045）：只有主人能设范围（`owner_only`）。“全局”包括家人的 Bot，确认页写出来。

“全局”可以当默认，因为能不能调工具由确认卡片决定（ADR-0016、ADR-0024），不由看不看得到决定。范围管的是 Bot 的分工，
和上下文里别塞进用不到的工具。技能没有卡片，所以带脚本的技能在确认页醒目提示（第 4 节）。

**开关放在两处，改的是同一份数据：**
- **插件详情页：** “给哪些 Bot”，显示“全局”或勾选的 Bot，可以改。
- **Bot 资料页：** 这个 Bot 能用的工具插件和技能，每一项写来源（“全局”或“指定给它”），可以单独关。
  单独关掉的项写“只对这个 Bot 关了”，让主人看得出是哪一层在起作用。

### 6. 升级、卸载、同名

- **升级：**
  - 走同一个确认，列出新增的工具（ADR-0024）、新增的技能、新增的必填密钥（ADR-0064），和带脚本的技能。
  - 升级保留旧版时，输入密钥及修改时间一并恢复；新版已保留而后置对账失败时，不回写旧密钥。
  - 范围不变。
  - 技能是只读挂载，Agent 改不了，所以不会有“本地改过的技能被覆盖”的问题。主人想改一个技能，就点“复制成我的技能”，得到一个上传来源的
    新包（ADR-0031），原来那个照常升级。
- **卸载：**
  - 从所有 Bot 的子目录里撤掉技能，从总线上撤掉工具，按“保留数据”处理密钥（ADR-0064）。
  - 被本批之外的插件依赖时照 ADR-0053 拦下（409，同 ADR-0039）；同批删除使用方和提供方时先撤使用方，再撤提供方。
- **同名：**
  - 工具在总线上带插件前缀，不会撞。
  - 技能的目录名就是它的名字（Agent Skills 规范要求两者一致），不能改名加前缀。两个插件给同一个 Bot 同名的技能时，
    后启用的那个对这个 Bot 不生效，Bot 资料页写“和 X 的技能同名，没有启用”，主人选留哪个。不按来源排优先级悄悄遮住。
  - 和 Bot 自己写的技能同名时，谁生效由那家 Agent 的优先级决定（第 4 节的表：挂在 Claude Code 企业目录的我们的优先，其余几家 Bot 自己的优先）。核心看不到 Bot 自己的目录，判断不了，所以不提示；Agent 插件页写明这家的规则。

### 7. 谁装的、副作用账本

- **谁装的：** 安装记录写明是主人在哪台设备装的，还是哪个 Bot 经卡片装的，详情页显示。
  Bot 自己写的技能留在它自己的目录里，核心不管。Bot 想把它变成能给别的 Bot 用的技能，就用现有的 `wonton__check_plugin`
  把它打成包，经卡片装，装好后记作“由 Bot X 安装”。
- **副作用账本：**
  - 每个插件一份。核心记下安装在容器之外做过的每件事：密钥、给了哪些 Bot、技能挂在哪些 Bot 的目录、能力绑定、
    别名、OIDC 客户端、Matrix 账号、局域网端口。
  - 卸载照着账本逐项撤销。卸载确认页从账本列出“会删掉什么、会留下什么”。
  - 账本写不进去时，安装失败，不装一个记不清的插件。

### 8. 目录：可以是插件，只给数据

- 商店的条目来自索引。官方索引（wonton-store）加“工具”和“技能”两区。条目可以由转换脚本从官方 MCP Registry 和 Docker MCP
  Catalog 生成：Docker 的 `mcp/*` 镜像按摘要固定；npm、PyPI 的包要写死版本和完整性哈希，运行时不 `npx` 拉最新的。
- 第三方目录可以做成插件，提供能力 `catalog`。它只给条目数据：名字、说明，以及 ADR-0031 的来源（仓库、提交、sha256）。
  - 取包、校验、预览、确认都由核心做。
  - 目录插件不能决定核心带什么凭据去访问哪个主机。
  - 它的条目在商店里标成“来自第三方目录 X”。

## 理由

- **runtime 和 kind 分开：** 一个 MCP 工具可以是容器，也可以是远程服务器；一个 Agent 插件也可以带技能。
  把形态塞进 `kind`，标签和执行就混在一起了。
- **用 Agent Plugins 1.0：** Codex、Claude Code 的插件和技能生态已经用这个格式，Wonton 不再造一个。字段放在它留好的
  `extensions` 里，不和别家冲突。
- **连接器做成插件：** 外连、OAuth 和令牌刷新是一整块会变的东西，现成的开源实现很多。放在可替换的插件里，核心保持薄
  （ADR-0015），出网和资源限制也能按插件给。
- **每个 MCP 服务器一个容器、不做聚合 hub：** hub 里的几个第三方服务器共用文件系统和环境变量，一个被攻破就能读到别的密钥，
  出网规则、资源上限和确认也只能按整个 hub 给。
- **技能挂一个目录、路径由 Agent 插件声明：** Docker 不能给运行中的容器加挂载，所以每个 Bot 挂一次目录、以后只改内容。各家 Agent 读的目录不一样，也没有统一标准，由 Agent 插件声明挂在哪里，核心不用认识每一家。
- **范围放在核心、默认全局：** 家里一般一两个 Bot，每装一个都要选是负担；按项目、按 Agent 的工具（vercel-skills、
  add-mcp）默认最窄，是因为它们没有确认卡片，看得到就能调。Wonton 的安全边界在卡片上，范围只管分工。
- **不允许本地改技能：** Hermes 用哈希保护本地改动，要 diff、reset、force 三个动作；只读挂载加“复制成我的技能”，
  就没有这个状态。

## 代价与风险

- 四种来源格式（compose、Agent Plugins、MCP Registry、Docker Catalog）都要维护清单校验和转换。官方 MCP Registry 是预览版，可能清空数据。
- OAuth 依赖一个公开的静态页面和一份 CIMD 文档，放在我们的 GitHub Pages 上。页面坏了，所有 Wonton 都连不上新的 OAuth 服务器。
- CIMD 的 `client_id` 就是这份文档的地址，刷新令牌绑定在它上面。授权服务器可能在刷新时重新取这份文档。所以：
  - 换地址时，旧文档永久保留；
  - 连接器按令牌记下 `client_id`，刷新用记下的那个，新授权才用新地址；
  - 旧回调页可以删，刷新不用回调。

  只支持动态注册的服务器，换地址后要重新注册，也就是重新授权一次。项目 Pages 的地址跟着仓库名走，用上以后文档仓库不改名、不转移。
- GitHub 官方 MCP 第一版只能填个人访问令牌。
- 技能能不能热加载要逐个 Agent 实测；不能热加载的 Agent，启用技能后要等下一次会话。
- Claude Code 挂在企业目录：和 Bot 自己写的同名技能撞上时，我们的优先；它的企业目录改动能不能中途生效还没核实。
- 各家 Agent 改了读取目录（比如 Claude Code 以后读 `~/.agents/skills`），要跟着改对应 Agent 插件的 `skills.mount`。
- 默认“全局”时，家人的 Bot 也会看到主人装的工具和技能，只是调用仍要主人确认。
- 远程服务器改了工具也照样放行：插件开了“始终允许”或 Bot 开了“一律允许”时，对方新加的工具、改了描述的工具不经确认就能被调用。主人装的时候信任的是这台服务器，不是当时那份工具清单；和 ADR-0024 对社区 MCP 的做法一致。
- 运行形态多了，核心要给每种都做校验、失败分类和 App 详情页。执行交给插件（连接器）后，增加的主要是这些。

## 没选的做法

- **默认装给最少的 Bot：** 外部工具这么做，是因为它们没有确认卡片。Wonton 里大多数主人只有一两个 Bot，每次都要选。
- **核心直接连远程 MCP：** 核心会变成 SSRF 跳板，OAuth 和令牌刷新进了核心，也换不掉。
- **聚合的 MCP hub（一个容器跑很多 MCP 服务器）：** 隔离、上限和确认都只能按 hub 整体给。
- **`kind: skill`：** 把形态和业务标签混在一起。
- **核心按 Agent 种类决定技能挂哪里：** 核心要认识每一家 Agent，违反 ADR-0015。
- **一律挂到 `~/.agents/skills`：** Claude Code 不读，Hermes 默认不读；容器里用 `npx skills add -g` 装技能也会因为只读而失败。
- **挂到 Agent 自己的技能目录（`~/.claude/skills`、`~/.hermes/skills`）：** 只读挂上会挡住 Agent 自己写、自己装的技能。
- **按技能逐个挂载：** 每次启用都要重建 Agent 容器。
- **像 Hermes 那样允许本地改动，升级时比较哈希：** 多出 diff、reset、force 三个动作和一个“改过了”的状态。
- **安装器做成插件（像 HACS、Koishi、BRAT）：** 安装要用 Docker socket，还要走核心的确认，交给插件就得给它 root。
  容器隔离的系统（Umbrel、Start9、CasaOS）都没有这么做。

## 主人已定（2026-11-08）

1. **技能和 MCP 装给谁：** 安装时选全局或指定的几个 Bot（第 5 节）；全局的技能按各家 Agent 原生读取的目录挂进去，Agent 自己会读（第 4 节）。Bot 经卡片装的只给它自己。
2. **远程 MCP 的 OAuth 第一版就做**（第 3a 节）：用 wonton-docs 的项目 Pages 放回调页和 CIMD 文档，回调页跳 `wonton://`。开 Pages 主人已同意，和连接器一起上。
3. **远程 MCP 改了工具照旧放行**（第 3 节），不要求重新确认。
4. **GitHub 的官方 MCP 先用个人访问令牌**（`input` 密钥），OAuth App 以后再说。

## 实施顺序

1. 接口 PR：
   - `runtime` 和 Agent Plugins 的 `extensions` 命名空间；
   - 范围的协议类型；
   - 规范 0.10，wonton-docs 同时提。
2. 按 Bot 放行工具：在总线已有的按 Bot 认调用方之上加范围，以及 Bot 资料页的开关。
3. 技能形态：存放、按 Agent 声明的目录挂载，官方 Agent 插件逐个实测发现和热加载。
4. 远程 MCP 连接器：用 rmcp 写薄连接器，做成系统插件；同时合 wonton-docs 的回调页和 CIMD 文档，开 Pages，跑一次 Notion、Linear 的 `/authorize` 验证。
5. 商店的“工具”“技能”两区，以及 MCP Registry、Docker Catalog 的转换。
6. 第三方目录插件（`catalog` 能力）。

## 补充：远端 MCP 的清单、白名单和接口（2026-11-13）

第 1、3、3a 节的细则。第 2 节的包格式第一版不用，第 4 节照旧。

- **清单写在 `x-wonton.remote_mcp`，不用 Agent Plugins 1.0 的 `mcp.json` 加 `extensions`。** 核心要先单独解析、校验这一形态，
  而容器插件本来就用 compose 里的 `x-wonton`，一条解析路径就够。Agent Plugins 1.0 的包留给第三方包和转换脚本，
  换过去时清单字段不变，只换外壳。
- **配置随每个请求带给提供方，提供方不读清单。** 核心发往提供方的每个请求带 `X-Wonton-Plugin`（插件 ID）、
  `X-Wonton-Remote-Url`（展开密钥后的服务器地址）、`X-Wonton-Remote-Auth`（`none`、`header`、`oauth`）、
  `X-Wonton-Remote-Forward`（清单 `headers` 填好密钥后的头，JSON 对象做 base64）；`X-Wonton-*` 由核心删掉重写。
  提供方不缓存清单，装、改、卸都不需要重启它。托管形态因此不必有包内目录，`HostedForm.mount` 可空，
  提供 `remote_mcp` 的插件不写 `provides.remote_mcp.mount`。
- **地址模板只允许在路径和查询里引用 `{secret:<名字>}`，每项必须是已声明的 `input` 密钥。** scheme、主机、端口、
  用户信息中不允许模板；校验时先用占位值替换模板，再检查 HTTPS、无用户信息、无片段，主机不是 IP 字面量、
  `localhost`、其子域或单段名。替换时把密钥按 URL 组件百分号编码，不能让密钥改变路径或查询的结构。
  在 URL 解析归一化之前拒绝 `.`、`..` 路径段（含 `%2e` 大小写变体），密钥与固定片段拼出的点段也一样拒绝；查询参数不受此限制。
  核心每次请求重新读取密钥，URL 和请求头的新值在下一个请求生效，不重建容器。
  可选 `input` 未填或清空时省略引用它的整个请求头，不发送空的鉴权值；URL 中引用的密钥不能省略。
- **展开后的地址只交给连接器握手和总线。** 握手请求体的 `url` 和总线的 `X-Wonton-Remote-Url` 使用展开值；
  核心日志、错误、详情和审计只用清单模板原文。握手和运行期的 HTTP、JSON-RPC、SSE 错误在进入 MCP SDK 前脱敏，
  展开地址换回模板，原文和编码后的密钥按同位置最长匹配移除；替换结果不再次参与匹配，保留非敏感上下文和错误分类。
- **装时握手是 `POST /_wonton/probe`。** JSON 请求体是 `url`、`auth` 和展开后的 `headers` 对象；返回分类 JSON：
  `outcome`、可选 `http_status`、不含密钥的 `detail` 和可选工具名列表 `tools`。`ok` 和 `legacy_sse` 都继续安装。
  旧式 HTTP+SSE 由连接器转发为核心使用的 Streamable HTTP，不因传输旧而拒装。
- **回调页用 wonton-docs 的 Pages 地址（第 3a 节）。** wonton-docs 先转到 wontonapp 组织再用，回调页和 CIMD 文档一起合。
- **两种新的失败类别：** `remote_unavailable`（连不上：DNS、TLS、超时、拒连）和 `remote_error`（远端回 5xx 或协议错误）。
  不复用 `TransportUnavailable`、`Upstream`，那两个说的是家里的服务器。
- **经出网代理时接受 DNS 重绑定的残留风险：** 连接器在每次连接前解析并检查地址，代理侧的解析结果和它可能不同；
  第一版按此接受，写在连接器的 PR 里。
- **填了密钥仍被远端拒绝就拒装**，错误写“服务器不接受填的密钥”，不装一个连不上的插件。
- **第一版不支持局域网远端 MCP：** `https` 地址且解析后是公网地址；要连内网就等 ADR-0019 的 `lan` 接进来。
- **两个接口 PR 和连接器、静态托管插件分别建公开仓库（`wontonapp/wonton-plugin-mcp-connector`、
  `wontonapp/wonton-plugin-static-host`）。** 连接器是系统插件，可替换。
- **两个接口 PR（清单和托管、连接器）之间不发版，共用尚未发布的规范版本 0.15。**

## 关联

- ADR-0006 和 ADR-0016（MCP 总线、按 Bot 白名单）
- ADR-0015（小组件与标准接口）
- ADR-0019（局域网）
- ADR-0021（公网入口）
- ADR-0023 和 ADR-0039（出网）
- ADR-0024（放宽确认）
- ADR-0031（插件来源）
- ADR-0033（Bot 装插件）
- ADR-0034（`wonton-help` 技能）
- ADR-0036（高权限）
- ADR-0037（资源上限）
- ADR-0045（家人和一个插件多个 Bot）
- ADR-0053（能力）
- ADR-0064（用户填写的密钥）
