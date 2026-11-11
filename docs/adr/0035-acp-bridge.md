# ADR-0035 ACP 桥：Matrix 与 ACP Agent 的连接形态

- 状态：已接受（2026-11-05）。表单模式的 `elicitation/create` 回 `decline`，转成多步卡片的提议见 [ADR-0038](0038-multi-step-cards.md)，M1 不做

## 背景

Wonton 要把 Codex、Claude Code、omp、Qwen Code、Kimi Code 等 Agent 作为 Bot 接入，但核心不能认识某一个 Agent。Matrix 是聊天、历史和端到端加密的边界；ACP 是 Agent 的边界；MCP 总线是工具的边界。现成的 [matrix-acp-bridge](https://github.com/chadvoegele/matrix-acp-bridge)（MIT，TypeScript）已覆盖普通消息、E2EE、恢复、typing、steering 和 ACP stdio，其 Matrix/ACP 分层和加密测试可作参考；但它的默认权限处理器会自动选择 `allow_always`，没有 Wonton 的主人确认通道，不能直接装入核心。桥因此自己实现 Wonton 的 Matrix/ACP 边界，不重写核心的 Matrix 加密库，也不把该项目的权限模型带进来。

范围只覆盖 C1–C3：消息、工具、权限、会话恢复和登录。执行轨迹、通用 Agent 框架、多传输和插件 DSL 不在范围内。

## 决策

### 组件和容器

ACP 桥是每个 ACP Agent 插件的一部分，不进核心，也不在核心里写 Agent 特例。插件提供两个服务：`bridge` 和 `agent`，一个 Bot 一对；`bridge` 使用 `wonton-matrix`，`agent` 只运行该 Agent 的 ACP 程序。ACP stdio↔socket 转发器写在 Agent 镜像入口脚本里，由插件清单声明命令和 socket；socket 经专用共享卷只挂给两个服务、权限 0600，bridge 只连接它，不跨容器启动 Agent 进程。两服务的其余通信经插件私网，不把 ACP HTTP 当成已存在的标准。

两服务首先为了数据隔离，而不是只为故障隔离：`bridge` 只挂 `${WONTON_BOT_DATA}/bridge`，`agent` 只挂 `${WONTON_BOT_DATA}/agent`，两个目录不互相可见。bridge 目录存 Matrix crypto store、Bot 登录态、恢复密钥和房间→session 映射；agent 目录存 Agent 的 HOME、workspace 和官方登录态。Matrix access token 只在 bridge 服务的 environment 中引用，agent 不拿到 Matrix token、crypto store 或恢复密钥；Agent 自带 shell 因此不能读取这些密钥。

桥服务是公共的、版本固定的 Wonton ACP bridge 镜像层；Agent 服务是每个插件自己的层。桥层包含 Rust bridge 和 `wonton-matrix`，不包含任何厂商 Agent；它不依赖 `rmcp`，因为桥不实现 MCP。Agent 层只放上游程序或其适配器、配置和许可证；Claude Code 第一次启动时在自己的容器内下载未修改的官方程序，不把它打进公共镜像。适配器、Agent、桥都按上游版本和 digest 固定。

一个插件默认只有一个 Bot（现有 `bots.services` 模型）；一个 Bot 的多个房间共享同一个 Agent 进程，但一个房间一个 ACP session。M2 再实例化多个 Bot。桥监控 Agent 退出并把原因分成 Agent 未就绪、传输不可用和上游错误，不重启成假成功。

### Matrix 到 ACP

桥以一个普通 Matrix 设备运行，核心把 Matrix access token 只注入 bridge 服务。核心创建主人与 Bot 的加密私聊并把房间 ID 交给桥；桥只接受主人的消息，群聊和其他发送者按 Bot 权限拒绝。桥复用 `wonton-matrix` 的 Sliding Sync、设备密钥、交叉签名和恢复密钥，不自己实现 Olm/Megolm。bridge 的 crypto store、Bot 登录状态、恢复密钥和房间→session 映射在 `${WONTON_BOT_DATA}/bridge`，目录权限 0700；agent 的 HOME、workspace 和官方登录态在 `${WONTON_BOT_DATA}/agent`，Agent 看不到 bridge 目录。恢复密钥只在核心收集/保管的文件中出现，不进日志、环境变量或 Matrix 消息。加密初始化失败就让插件不健康，绝不退回明文。

收到普通文本后，桥按房间查 session；没有就调用 `session/new`，随后切到插件配置的模式（见下文）。文本成为一个 `session/prompt`，ACP 的 `agent_message_chunk` 只在该轮结束后拼成一条 Matrix `m.text`；中间不展示 thought、tool call、usage、plan 或执行轨迹，只发送 typing 状态。图片和文件 C1 不转，收到时返回明确的输入不支持错误。桥限制输入、输出、队列长度和单轮时间，超限报错误，不截断成成功。

同一 session 同时只有一个 `session/prompt`。默认排队，队列满返回 busy。主人发送“打断重来”操作时，桥先 `session/cancel`，等原 prompt 的响应（包括 `cancelled`）后再用新文本调用 `session/prompt`；取消后的迟到 chunk 在该 prompt 的边界外丢弃。桥在 ACP `initialize` 的顶层 `_meta.steering.supported` 为真时支持现成的 `_session/steering`：进行中直接发送，返回 `injected`；空闲返回 `promptRequired`，再走普通 prompt。没有该能力就排队，不调用 Qwen 私有方法。Matrix 控制消息只由桥识别，不发给 ACP：`/wonton interrupt <文字>` 是打断重来，`/wonton queue <文字>` 是显式排队；普通聊天文本默认排队。

房间空闲达到 10 分钟时桥调用 `session/close`，释放 Agent 会话内存；这是本设计的回收决定。session ID、cwd 和 workspace 持久化在 `${WONTON_BOT_DATA}/agent`。下条消息按 Agent 能力选择 `session/load` 或 `session/resume`（必须带完整的 `mcpServers`），不支持或返回会话不存在时才新建并如实丢失旧上下文。容器重启时对每个持久 session 做同样的恢复；工作区不存在是插件错误，不创建一个空目录掩盖它。

### 权限请求和确认

Agent 的 `session/request_permission` 不是 Matrix 消息里的普通按钮，而是核心确认系统的一个请求。桥把 ACP 的 `toolCallId`、标题、说明、已收到的 `tool_call`/`tool_call_update` 参数和选项原样整理成核心可渲染的结构化卡片；同一 `toolCallId` 的增量合并，秘密字段和 token 永不进卡片、日志或审计。卡片发在 Wonton 私聊，包含 ACP 提供的选项名称，不让模型文字决定按钮。

核心复用现有确认卡片的通用 `approve`/`reject` 方向，不新增 `allow_always`、`reject_always` 或 `allow_window` 的 Answer 枚举。为让一张卡片保留多个 ACP 选项，卡片的按钮和主人的回答扩展一个 opaque `choice_id`，由桥为这次请求生成，核心只原样放到卡片上再原样带回，不认识 ACP `optionId`；回答对不上卡片上的按钮时返回 400 `invalid_input`。`allow_once`/`allow_always` 都显示为 Agent 给的同意选项，`reject_once`/`reject_always` 都显示为拒绝选项，桥再按自己的映射回原始 ACP `optionId`。`allow_always` 是 Agent 自己的作用域，不转成 `ApproveWider`，不写 Wonton `tool-policy`；Wonton MCP 工具的持续放行仍只按 ADR-0024，两个策略不互相扩大。若 ACP 没有拒绝选项，核心另加一个不带 `choice_id` 的拒绝按钮“取消”，桥回 ACP `cancelled`。核心审计决定，不让桥或 Agent 自己批准。

入口不写 ACP 或桥的特例，插件网络里的任何 Agent 适配层都能用：`POST /agent/permissions`，和 MCP 总线同一个地址（`WONTON_MCP_URL` 换掉路径）、同一道门（按来源地址认出调用方，要有 `tools` 权限），但不是 MCP，因为桥不说 MCP。它是长请求，用 HTTP 而不是回调，桥不用再开监听。请求带 `tool_call_id`、标题、说明、参数和选项（`choice_id`、`kind`、显示名），响应是三种结果之一：`selected`（带 `choice_id`，包括主人选了拒绝类的选项）、`cancelled`（主人点了核心加的“取消”）、`expired`。选项不认识、没有选项或 `choice_id` 重复时返回 400 `invalid_input`；已经有太多请求在等主人时返回 429 `too_many_pending`，和 MCP 的卡片共用同一个上限（ADR-0016）；卡片发不出去时返回 503 `confirmation_unavailable`。桥断开长请求就是撤回，卡片随之作废，主人的回答不再算数。选了同意类的选项后，卡片写明选了哪一项，不显示“正在执行”，因为执行归 Agent，核心看不到结果。类型在 `wonton-protocol` 的 `agent_permission` 模块。

桥在收到主人回答后以映射出的原始 ACP `optionId` 回答 `session/request_permission`；主人选择 ACP 的拒绝选项时保留拒绝语义，核心的取消动作、确认到期和主人取消该 prompt 都回 ACP `cancelled`。Agent 等确认期间，ACP 请求和核心卡片的 TTL 都是 10 分钟。桥在取消、超时、Agent 断开时清理 waiter。工具已经开始执行但连接断了，桥向聊天报告“不确定”，不报告失败。

ACP Agent 自带的 shell、文件和终端只作用于 Agent 容器的 workspace，仍走 ACP `session/request_permission`。为避免 Agent 调 Wonton MCP 工具时出现两张卡，每个 Agent 插件应在 Agent 自己的配置里只放行 Wonton MCP 这个服务器；具体配置键和写法在 C2 接入 Claude、Codex 或其他 Agent 时逐个核实。桥不按工具名自动回答；某个 Agent 做不到时如实保留两张卡。经 Wonton MCP 总线的确认只归核心一张卡，Agent 的 ACP 允许不替代它。

Agent 仍是不可信执行者：插件清单授权的共享目录才可挂载，宿主 root、host network、特权模式一律没有。桥不声明 ACP `fs`、`terminal`、`subagents`，不把宿主目录、Docker socket 或额外文件夹交给 Agent。

### MCP 总线

`session/new`、`session/load`、`session/resume` 的 `mcpServers` 只放核心给该 Bot 的 Wonton MCP 入口；插件工具已经由核心总线聚合，不让 Agent 直连各插件。桥不实现 MCP，只把这个入口作为 ACP session 配置传给 Agent。`rmcp` 继续只由 `wonton-core` 使用，因为核心需要它实现 Wonton MCP 总线；bridge crate 不增加这项依赖。核心提供 Streamable HTTP；桥在 Agent 声明 `agentCapabilities.mcpCapabilities.http` 时传 `type: "http"` 的 Wonton 服务器。实测 `omp 18.16.0` 宣布 HTTP/SSE，并在 `session/new` 收到 HTTP `mcpServers` 后发出 MCP `initialize`、`notifications/initialized` 和 `tools/list`。

只收 stdio 的 Agent 不由桥跨容器起进程：Agent 服务的镜像层自带固定版本、MIT 许可的 `mcp-remote`（或等价的最小 stdio→Streamable HTTP 转发器），桥传入 `type: "stdio"`、绝对命令路径和不含秘密的 Wonton URL；ACP Agent 在自己的 Agent 容器内启动这个命令。若 Agent 镜像不能提供该命令，session 建立失败并显示插件未就绪。

Wonton MCP 入口按插件网络来源授权，C1–C3 不引入 bearer token；`mcpServers` 里没有凭据，URL 也不含凭据，所以不会进 ACP 日志、会话历史、命令行或 Matrix。Agent 自己的官方登录态留在 `${WONTON_BOT_DATA}/agent`，不作为 MCP headers 传递。以后若核心改成 token，先加只读 secret-file/FD 合同，禁止把 token 放 URL、命令行、环境变量或日志；本 ADR 不设计外部 MCP 凭据代理。MCP 的 `allow_window`、始终允许和只读策略仍由核心按 ADR-0024 判定，不通过 ACP 权限映射自动开启。工具结果是不可信输入，桥不解释结果、不据此自动批准权限。

桥不声明 `fs`、`terminal`、`subagents` 等 ACP client capabilities，也不提供第二套工具面。每个 Agent 插件显式配置它的最严格可用模式；已实测的例子是 Codex `read-only`、Qwen `default`，其他 Agent 在自己的插件中填入经过验证的模式。桥在 `session/new` 后立即切模式：应答带 `modes` 时用 `session/set_mode`，只有类别为 `mode` 的 `configOptions` 时用 `session/set_config_option`，两样都没有（如 dsh 0.2.2）就不分模式、跳过；没有聊天入口切到 bypass/yolo/full-access。

### 登录（C3）

模型 key 仍走 magpie；官方账号只在 Agent 官方程序内登录。插件清单用 `agent.login` 声明支持的方式并申请 `network.egress`，桥不把订阅请求送进 magpie。ACP 的 URL 引导（`elicitation/create` 的 URL）转成 Matrix 私聊中的链接卡片，主人点击后回到官方页面；终端类登录把官方给的链接和提示发到聊天，主人回复的授权码写入该 ACP 进程的 stdin。主房间是群（共享 Bot 的主房间在家人群）时，登录不在群里进行：桥在群里只回一句，方式列表、链接、码和取消都在主人与 Bot 的私聊里，找不到私聊就请主人在私聊里发起。登录状态是全 Agent 一份，同一时间只有一个。技术决定是直接转交、默认开启且不加开关；授权码不进日志、不存盘，聊天里的回复消息照常留在加密房间（密钥消息撤回），群里的那一句回执留在群里。公开版本不等 Anthropic 书面确认；条款怎么读、风险在谁见 [ADR-0034](0034-onboarding-and-help.md)，不在本 ADR 重复。

## 形态取舍

### 一个公共桥容器还是每个 Agent 自带桥

- **同一公共桥服务，多个 Agent 子进程**：镜像少、内存省；但 Agent 进程、依赖和数据边界混在一起。
- **每个 Agent 插件自带一份桥**：边界清楚、插件可独立升级；镜像和常驻进程略多。
- **决定**：每个 Agent 插件一份桥服务，每个 Bot 一对 bridge/agent；两服务的首要理由是 bridge/agent 数据目录隔离，故障隔离是附带收益。

### 代码、依赖和镜像

桥代码放主仓库私有的 `crates/wonton-acp-bridge`，因为它复用私有 workspace 的 `wonton-matrix`；插件仓库是公开的，不能复制或依赖主仓库源代码。主仓库 CI 构建多架构 `ghcr.io/wontonapp/wonton-acp-bridge:<version>`，发布后按 digest 固定；公开插件仓库只放 Compose、Agent 层、配置和 notices，并引用这个 digest；ACP Agent 插件合在一个仓库、每家一个子目录，见 [ADR-0079](0079-agent-plugins-monorepo.md)。推荐这个边界；把桥源代码另开公开仓库需要先抽出 Matrix 边界和单独处理许可证，不属于 C1。

桥用 Rust，原因是仓库已有 Matrix 的 Rust 边界、厚核心的错误分类和低常驻开销；桥是独立插件进程，不能调用核心内部 API。桥及其复用的 `wonton-matrix` 随私有主仓库的 `LicenseRef-Proprietary` 发布；公共插件仓库只引用已构建的镜像，不暴露桥源代码。桥不说 MCP，`rmcp`（Apache-2.0）只留在已实现 MCP 总线的 `wonton-core`；桥及其依赖不引入 GPL/AGPL。Agent 的专有许可证、适配器许可证和上游 notices 随 Agent 插件清单分发；不能满足某个 Agent 的许可证时不上架。

每个活跃 ACP session 的内存预算按实测值留余量：Claude 约 200 MiB/会话增量，Qwen 有 2.7–2.9 GiB 的瞬时多会话峰值；插件清单声明限制，Qwen 默认单 session 并设容器内存上限，超限显示插件被杀/未就绪，不静默重试。空闲 10 分钟后 `session/close`，不删除持久状态。

## 验证结果与未决项

已实测条件分两组：ACP 实验使用 `node:24-bookworm-slim`、macOS + OrbStack（Docker 29.8，arm64）、开发机 magpie 的网关默认模型，Agent 版本为 Claude `claude-agent-acp` 0.98.0、Codex `codex-acp` 2.3.0、Kimi Code 2.4.0、Qwen Code 0.26.2；覆盖 `session/new`、续聊、`session/load`/`session/resume`、cancel、权限允许/拒绝/取消、只连模型网关、Claude/Codex 的 `_session/steering`、模式和内存。另在本机直接运行 `omp 18.16.0`，用临时本地 MCP HTTP 服务验证 `session/new` 收到 HTTP `mcpServers` 后实际发出 MCP `initialize`、`notifications/initialized` 和 `tools/list`；未启动完整 Wonton 栈。

未实测：各 Agent 只放行 Wonton MCP 服务器的具体配置键和写法；Codex 的 `model_catalog_json` 警告修复；Qoder 的 session；完整账号登录和登录态重启；C3 条款要求。C2 首个实现用 `omp 18.16.0` 接 magpie、无需账号；主人的验收是 omp 加 Codex 或 Claude Code 至少一个，不以 Qwen 或 Kimi 作为验收组合。

C2 首版随后在开发机的 Docker 29.7.2、核心 `1f2c3be`、Tuwunel 1.9.4 上按插件安装并实跑：核心代 Bot 建加密私聊，主人客户端发消息后，桥建立 ACP session 并把 HTTP `mcpServers` 交给 `omp 18.16.0`；OMP 实际完成 MCP `initialize` 和 `tools/list`，并经 `WONTON_MCP_URL` 调用了核心只读工具 `wonton__list_plugins`，结果回到聊天。确认接口用假 ACP Agent 驱动：允许和拒绝分别映射回原 `optionId`，桥断开时请求撤回、卡片从待确认列表消失。真实 OMP 的 `/wonton interrupt` 先取消旧轮再发新 prompt，`/wonton queue` 在有活动轮时不提前发送。Agent 与桥同时重启后，`session/load` 先等回放更新安静，再发新 prompt，未再被立刻取消。Agent 容器只挂自己的数据目录和 socket，环境里没有 Matrix 令牌；bridge 数据目录为 0700。平台原因未完成的三项：OMP 到 magpie 返回 `上游错误：Internal error`，所以 OMP 的 shell 权限卡片和真实模型回复未验收；开发机容器到宿主 3425 的模型通路未打通；App 内点带 `choice_id` 的按钮另测。

## 要改核心的部分

- `crates/wonton-protocol`：给 `CardChoice`/确认回答增加兼容的 opaque `choice_id` 选择字段，使一张卡片能区分多个同为 approve 或 reject 的 Agent 选项；保留现有 `Answer` 方向和 `Unknown` 兼容规则，`allow_window` 仍属于 ADR-0024 的 MCP 工具策略。
- 插件规范 v0.6：补充 Agent 启动命令、最严格模式、`mcpServers` 能力、Agent 镜像入口的 stdio↔socket 转发命令和 `agent.login` 持久化/秘密约束；规范仓库的 PR 与核心 PR 同步。
- 核心插件渲染：允许 bridge/agent 作为一对 Bot 服务，bridge 只挂 `${WONTON_BOT_DATA}/bridge` 并在 environment 引用 Matrix token，agent 只挂 `${WONTON_BOT_DATA}/agent`；不把 Docker socket、宿主 root、Matrix token、crypto store 或恢复密钥交给 Agent。
