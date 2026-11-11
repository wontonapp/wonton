# ADR-0079 ACP Agent 插件：一个仓库、一个底座，每家一份配置

- 状态：已接受（2026-11-11）

## 背景

ADR-0035 定了每个 ACP Agent 插件 = 公共的桥服务 + 这家自己的 Agent 服务。ADR-0017 定了一个插件一个仓库。按这两条，第一批的
Codex、Claude Code、omp 各开了一个公开仓库；第二批的 Kimi Code、Qwen Code、OpenCode 照这样还得再开三个。

把这几个仓库比一比，会发现真正每家不同的很少：

- 桥不认识任何一家 Agent；`scripts/`、CI、`AGENTS.md`，以及 compose 里的 bridge 服务，各仓库几乎一样。
- `wonton-agent-base` 底座镜像用一份 `agent.toml` 描述“装哪个 CLI、ACP 怎么启动、网关钩子、终端登录、登录网页”，统一做 stdio↔socket
  转发和登录 socket。Claude Code 的 Agent 层用上了它，只剩配置和几个短脚本（约 350 行）；Codex 和 omp 的 Agent 层各写了一份入口（368 行、289 行），
  做的是同一件事（行数按 2026-11-11 的插件仓库统计）。
- 桥镜像按摘要固定在每个插件的 compose 里，桥升级一次，就要给每个 Agent 插件各发一个版本。

每家躲不开、要单独做的有九样：安装来源和摘要、ACP 启动命令、网关钩子（把 `WONTON_MODEL_*` 写成这家的配置格式）、实测出来的最严格模式、
只放行 Wonton MCP 的配置、登录方式和条款结论、skills 和提示词放哪个目录、清单文字（名字、简介、许可证、notices、图标、内存）、冒烟探针。

ACP 官方注册表（agentclientprotocol/registry，Apache-2.0）收录了主流 ACP Agent 的版本、许可证和带摘要的分发地址，但上面九样它都不管，
所以它只能当“新版本在哪”的数据来源，不能当运行时随便装的商店。

## 决策

### 1. 一个仓库，每家一个子目录，每个子目录是一个独立插件

- 公开仓库 `wonton-agents` 放三样东西：底座镜像（`wonton-agent-base` 并进来）、compose 生成器，以及 `agents/<id>/`。
- `agents/<id>/` 只放这家独有的东西：`agent.toml`、网关钩子、登录脚本、提示词和 skills、`agent.yaml`（清单文字、最严格模式、内存）、
  许可证和 notices、冒烟探针。
- 生成器把 `agent.yaml` 和公共模板合成这个子目录的 `docker-compose.yml`，生成结果也提交进仓库。CI 检查生成结果是不是最新的，
  并对每个子目录跑 `plugin check --official`；CI 按子目录分开跑，一家失败不挡别家发版。
- 每个子目录是一个独立插件包：插件 id、商店条目、版本、名字和许可证都分开，用户在商店里看到的仍是“Kimi Code”“Qwen Code”。
  一个仓库只是为了一起维护，核心不会因此把它们当成一个插件。
- 桥镜像的摘要写在公共模板里，桥升级只要一个 PR，再重新生成所有子目录。

### 2. 核心：插件来源可以带子目录

- 来源写成 `github:<owner>/<repo>/<子目录>@<tag 或提交>`。核心按提交取这个子目录下的 `docker-compose.yml` 和它引用的文件，
  解析和校验和根目录的插件包一样。引用的文件不能跳出子目录。
- 带子目录时，“最新版本”只看 `<子目录名>/v<语义化版本>` 这类 tag；不带子目录的来源仍按 `v<版本>`，行为不变。
- 商店的 `index.json` 每项多一个可选的子目录，`update.sh` 照此生成。
- ADR-0017“一个插件一个 Git 仓库”改成“一个插件包是一个仓库的根目录，或其中一个子目录”。

### 3. 版本来源

CI 机器人定时读 ACP 注册表，某家有新版本时提一个改 `agent.toml` 版本和摘要的 PR；冒烟通过才合并。注册表只给版本号、没有摘要的
（npx 类条目），由机器人自己从 npm 取 `dist.integrity`。不在注册表里的（比如 omp）手动升级。

### 4. 收录哪些

- 第一批 Codex、Claude Code、omp、dsh 并进来：Codex、omp 删掉自己的入口，改成 `agent.toml` 加网关钩子；Codex 的适配器换成
  `@agentclientprotocol/codex-acp`；Claude Code 原样搬过来；dsh（`geraldpeng6/wonton-plugin-dsh`）已是 ACP 加底座的结构，
  和 Claude Code 一样原样搬进 `agents/dsh/`。插件 id 不变，旧仓库归档，商店改指向新来源，不写迁移。
- 第二批 Kimi Code、Qwen Code、OpenCode。每家要实测最严格模式、网关钩子和只放行 Wonton MCP 的配置，通过冒烟才上架。
- Google 的命令行 Agent 不收录。Gemini CLI 已不面向个人账号；Antigravity CLI 是专有软件，条款把“配非 Google 的产品用它的服务”
  算作违约。条款或接法有变化时再写新 ADR。

### 5. 不做的

- 不做“一个插件、安装时选 Agent”：那样清单就成了动态的，许可证、登录、权限、图标都随选择变，商店没法再按摘要核对静态插件包。
- 不在桥或核心里写任何一家的特例；每家的不同只放在自己的 `agents/<id>/` 里。
- 闭源、条款没写能不能再分发的 Agent CLI，不打进镜像，由底座在第一次启动时从官方来源按摘要下载（和 Claude Code 一样）。

### 6. 自带 Matrix 的 Agent

Hermes、OpenClaw 用它们自带的 Matrix 客户端接入（`agent.protocol: matrix`），不经桥，也不用底座。已定（2026-11-11）：

- **保持一家一个仓库，不放进 `wonton-agents`。** 它们跑的是上游原版镜像，接网关的写法各不相同，用不到这个仓库共用的桥、底座和
  `agent.toml`；放在一起，桥或底座一改，就会挡住和它们无关的发版。它们直接说 Matrix，ACP Agent 经桥说 ACP，按 ADR-0015 是两类组件、
  两份合同。
- **不改走 ACP。** 两家上游都有 ACP 模式。换过去，Agent 自己的工具也能走确认卡片；但 Hermes 的 ACP 模式不能主动发消息、没有定时任务，
  OpenClaw 的 ACP 模式不接受会话里传入的 `mcpServers`。需要“每一步都确认”的 Hermes 时，再在 `wonton-agents` 里加一个子目录
  当另一种选择。
- **审批。** Hermes 用自己的 smart 审批，只有主人能关；OpenClaw 保持上游默认，不在聊天里问，靠容器隔离。经 Wonton MCP 的
  操作，两家都照样走 Wonton 的确认卡片。

## 理由

- 合乎 ADR-0015：桥和底座只认 ACP 和 `agent.toml` 这两份合同；换一家 Agent 只换一个子目录，核心和桥都不用改。
- 新增一家从“开仓库、配 CI、写入口”变成写一个子目录，主要是配置，Agent 也能一次写对。
- 桥、底座和所有 ACP Agent 在同一个 CI 里一起测，桥一改就知道哪家坏了。

## 代价与风险

- 核心多一种来源格式，还要按子目录选 tag；商店索引格式随之改。
- 用户从商店以外的地方安装时，要写带子目录的来源，比原来长。
- 一个仓库的权限和 CI 额度由所有 ACP Agent 共用。

## 关联

ADR-0007（Agent 与 Bot）、ADR-0015（小组件与标准接口）、ADR-0017（插件仓库和商店，本篇修改它的“一个插件一个仓库”）、
ADR-0034（各家 Agent 的登录和条款）、ADR-0035（ACP 桥）、ADR-0059（任务话题）。
