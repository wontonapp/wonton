# ADR-0128 Bot 干活轨迹：状态消息旁加一份工具调用清单，由 Agent 插件从上游观察钩子发

- 状态：提议（2026-11-27，主人定做小样；谁能看、显示多少等见“要主人拍板”）
- 补充 [ADR-0082](0082-agent-run-status.md)：状态消息旁边多一个 `app.wonton.trace`；第 3 节“`step` 只按类别写”不变，
  逐个工具调用的说明放进清单，按第 3 节的隐私边界写
- 修订 [ADR-0007](0007-agents-and-bots.md)“默认 Agent 只收发消息，不做执行轨迹和工具过程展示”：工具过程按本篇显示，
  正文仍是一轮结束时发完的一条

## 背景

Bot 干几分钟活时，主人看不出它读了哪个文件、跑了什么命令、调了哪个应用的工具、成没成。要求：

- 只读旁路：不往 Agent 的上下文里加东西，不多花模型 token，不改上游 Hermes 源码（[ADR-0108](0108-hermes-default-kernel.md)）。
- 数据从现有的标准接口或上游正式接口来。
- 命令参数、文件路径可能带密钥或私人信息，写清显示多少、谁看得到，默认收起。

参考 CrommaCode 的做法：Agent CLI 的工具前钩子往一个 JSONL 文件追加 `{at, cli, tool, path, cwd}`，App 只读这个文件，
不改 Agent、不花 token。

## 查过的来源

按 Hermes `v0.23.4`（`nousresearch/hermes-agent@sha256:ef0650d5…`，2026-11-27 在镜像里查）：

| 来源 | 够不够 |
| --- | --- |
| ACP `session/update` 的 `tool_call`、`tool_call_update` | 字段够：`toolCallId`、`title`、`kind`、`status`、`locations`。但只在有 ACP 客户端驱动 Hermes（`hermes acp`）时才发。Wonton 上 Hermes 走原生 Matrix，ADR-0108 删了 Matrix↔ACP 桥，ACP 只剩 Hermes 调别的 Agent 那一侧（Hermes 是客户端）。要拿到就得重建桥，ADR-0108 已经否了。所以用 ACP 的词表，不用 ACP 的传输。 |
| MSC4564 状态消息（ADR-0082） | 格式和 App 的显示都有，但 Hermes `v0.23.4` 不发（镜像里没有 `org.fluxchat.agent`、`m.agent`）；而且 MSC 只有一句 `step`，没有逐个调用的清单。 |
| Hermes Matrix 的 `display.tool_progress` | 上游原生，但发的是给人看的纯文字（图标加参数预览），客户端只能猜着解析；不分成败；Hermes 插件为了“只收发消息”关着它。 |
| Hermes 插件钩子 `pre_tool_call`、`post_tool_call` | 上游正式的插件接口，Hermes 插件已经在用同一套（`pre_llm_call`、`on_session_end` 等）。带 `tool_name`、`args`、`session_id`、`tool_call_id`，`post_tool_call` 另带 `status`（`ok`、`error`）和 `duration_ms`。返回 `None` 就是只看，不改参数、不改结果、不进上下文。 |
| 核心 MCP 总线的审计（ADR-0016） | 只有经 Wonton MCP 的调用，看不到 Hermes 自带的终端、文件、浏览器工具；而且审计在核心，不在聊天里。 |

## 决策

### 1. 来源：Agent 插件挂上游的观察钩子，词表照 ACP

- Hermes 插件挂 `pre_tool_call`（记一项“进行中”）、`post_tool_call`（按 `status` 改成“完成”或“失败”）和
  `on_session_end`（这一轮结束）。钩子只读，返回 `None`，不调模型，不碰工具的参数和结果。
- 每项的字段照 ACP 的 `ToolCall`：`id`（`tool_call_id`）、`kind`（ACP `ToolKind`）、`title`、`status`（ACP `ToolCallStatus`
  里的 `in_progress`、`completed`、`failed`）。`kind` 和 `title` 用上游 ACP 适配器同一套函数算
  （`acp_adapter.tools.get_tool_kind`、`agent.display.build_tool_preview`）：和 Hermes 经 ACP、在终端里显示的是同一句。
  这句预览不脱敏终端命令（`v0.23.4` 实测 `Authorization: Bearer …` 原样留着），插件再整行过一遍上游日志用的
  `agent.redact.redact_sensitive_text`，然后才截短。这几个不是插件接口，插件的上游契约测试钉住它们，升级 Hermes 时先跑。
- 只记这一轮自己的调用：按 `session_id` 对上 Matrix 来的那一轮（插件已有的来源记录），对不上的（定时任务、网页控制台、
  委派出去的子代理）不记。
- 客户端不认 Hermes：别的 Agent 发同样的格式，显示一样（[ADR-0015](0015-small-components-standard-interfaces.md)）。

### 2. 格式：扩展 ADR-0082 的状态消息，不另起事件

一轮第一次调工具时发 ADR-0082 的状态消息（`m.notice`，带 `org.fluxchat.agent`），状态对象旁边加 `app.wonton.trace`，
和 `app.wonton.end` 一样放在状态对象外面：MSC 的对象归提案，不往里塞我们的键。

```json
{
  "msgtype": "m.notice",
  "body": "正在执行命令（第 2 步）",
  "org.fluxchat.agent": {"v": 1, "run": "$request", "state": "working", "step": "执行命令", "steps": 2},
  "app.wonton.trace": {
    "v": 1,
    "calls": [
      {"id": "call_1", "kind": "read", "title": "read_file: notes.md", "status": "completed"},
      {"id": "call_2", "kind": "execute", "title": "terminal: ls -la", "status": "in_progress"}
    ]
  }
}
```

- 推进和结束照 ADR-0082：用 `m.replace` 编辑，`m.new_content` 带全量清单；状态值变了立刻编辑，只有清单变了的最多每 3 秒一次，
  一轮最多 60 次，到了只等最终那次。结束时 `state` 是 `done` 或 `failed`，主人打断的加 `app.wonton.end: cancelled`。
- 清单只留最近 30 项，`title` 最长 120 个字符；`steps` 是这一轮的总数，比清单长就是前面的截掉了。单条事件远小于 Matrix 的 64 KiB。
- 没调工具的一轮不发状态消息：只聊天的一轮不多一行。
- `body` 是按说话人语言写的一句话（“正在执行命令（第 2 步）”“完成，用了 5 步”），不带 `title`。不认识这个格式的客户端
  （Element、旧版 App）只看到这一行，编辑后是最新的一句（ADR-0082 第 5 节）。
- 插件重启：还没结束的状态消息记在 Bot 数据目录里，重启后网关第一次收到消息时编辑成 `failed` 加
  `app.wonton.end: uncertain`，不留一条永远在干活的消息。上游没有“网关连上了”的插件钩子，所以要等一条消息；
  在那之前 App 照 ADR-0082 第 4 节按 Bot 的在线状态显示。
- `v` 不是 `1` 或 `calls` 不是数组时，客户端当没有清单；缺 `id`、`kind`、`title`、`status` 的项跳过；不认识的 `kind`、`status`
  落到 `Other`。

### 3. 隐私：显示多少、谁看得到

- **谁看得到**：状态消息是房间里的普通事件，这一轮所在房间的成员都看得到，和看得到这一轮回答的人是同一群人。
  房间不加密（[ADR-0118](0118-im-three-layers-push-with-body.md)），内容和聊天正文一样存在家里的 Matrix 服务器上。
  不另开只给 Bot 归属人的通道。
- **显示多少**：
  - 私聊（房间里只有 Bot 和一个人，按 Hermes 适配器的判法）：`title` 是工具名加上游的一行预览，命令、路径或查询，
    整行按上游日志的规则脱敏（API key、令牌、`Authorization` 头这类模式）后截到 80 个字符。不带工具的结果、文件内容、
    命令输出和完整参数。
  - 多人房间：`title` 只写类别的一句话（“读文件”“执行命令”），按说话人的语言；家庭群里别人看不到你让 Bot 读了哪个文件、跑了什么命令。
  - 房间人数取不到时按多人房间写。
- **默认收起**：状态行只显示当前一步或“N 步”，点开才露出清单。
- 推送不加：`m.notice` 默认不推，编辑不推（ADR-0082 第 4 节）。
- 锁屏和灵动岛（[ADR-0117](0117-bot-task-live-progress.md)，提议）不带清单，只用状态对象里的 `step`、`steps`：那条路的内容是明文。
- 只对核心认得的 Bot 账号解读（ADR-0082 第 4 节），别人发的同样内容按普通消息显示。

### 4. App 怎么显示

- 客户端核心把清单解析进 `TimelineContent::AgentRun` 的 `trace`：每项 `id`、`kind`、`title`、`status`。
- 一轮显示成不确定（Bot 离线，ADR-0082 第 4 节）或已经结束时，还标着进行中的项由核心合成成 `Uncertain`，壳不转圈：
  状态不确定就如实显示不确定。
- 壳只渲染：状态行下面一个默认收起的清单。`working` 行带一个展开箭头；`done` 行的“N 步”点开时，清单和 Bot 的总结一起露出。
  每项一行：按 `kind` 的图标加 `title`（Bot 的原话，不翻译）；进行中小转圈（减弱动态效果时不转），失败用错误色的标记，完成不加标记，
  不确定用不确定的标记。

## 考虑过的做法

- **重建 Matrix↔ACP 桥，直接转发 ACP 的 `tool_call`**：ADR-0108 刚删了桥；为了显示轨迹再养一套桥，代价远大于收益。
- **打开 Hermes 的 `tool_progress`**：上游原生、零胶水，但是纯文字、不分成败，每个工具一行，客户端只能解析给人看的字。
- **单独的事件类型（每次调用一个事件）**：别的客户端完全看不到，也没有 `body`；一轮几十个事件，和状态消息对不上。
- **每次调用发一条消息**：刷屏，每条都可能响铃。
- **写 JSONL 文件让核心读（CrommaCode 的做法）**：核心要去读插件的数据目录，破坏组件边界（ADR-0015），App 还要一条新接口拉；
  换 Agent 要重做。
- **核心新开接口收轨迹再下发**：核心为 Agent 加了一条非标准通道；Matrix 已经有加密、多设备同步和历史。
- **把清单塞进 MSC 的状态对象里**：对象的键归提案，提案以后加同名的键就撞了。

## 要主人拍板

1. **谁能看**：推荐这一轮所在房间的成员（和看得到回答的人一样）。备选：只给 Bot 归属人，要另开私聊或设备间消息发；
   家人问共享 Bot 的轨迹反而落到主人那里，涉及家人的隐私。
2. **多人房间显示多少**：推荐只写类别。备选：和私聊一样带预览；或多人房间不发清单。
3. **私聊显示多少**：推荐上游的一行预览（80 字符、脱敏）。备选：只写类别；路径只写文件名。
4. **要不要给开关**：推荐默认开。备选：插件设置里加一个“聊天里显示干活轨迹”的开关。

## 代价与待验证项

- 每轮有工具调用时多一条消息和几次编辑，加密房间里每次编辑都是一个事件。
- 依赖上游几个不是插件接口的函数，升级可能改名；契约测试在升级时报出来。
- 上游的脱敏是模式匹配，不保证认出所有私人信息；私聊里的预览仍可能带私人的路径或文件名。
- Hermes 经 ACP 发给别的客户端的 `title` 不过这层脱敏，是上游的事，不在本篇范围。
- MSC4564 还没稳定（ADR-0082），字段和前缀可能变。
- 同一轮里委派出去的子代理的调用不在清单里（会话号对不上）。

## 验证结果

2026-11-27，插件开发环境（核心 `main` 镜像、Tuwunel `v2.0.0`、Hermes `v0.23.4`、Hermes 插件 1.4.0，模型是开发机网关上的
DeepSeek）：主人在私聊里让 Hermes 跑两条命令、读一个文件。

- 第一次调工具时房间里出现状态消息“执行命令（第 1 步）”，清单里第一项 `in_progress`；之后每步一次编辑，最后编辑成
  `done`、“完成，用了 3 步”，三项都是 `completed`，然后才是回答。
- 命令里的 `Authorization: Bearer sk-…` 在 `title` 里是 `Bearer ***`。
- 客户端核心解析的是这次真发的内容（`wonton-matrix` 的 `what_the_hermes_plugin_sends_parses`）。

没在真实环境里验：多人房间只写类别（插件单元测试覆盖）；App 里的显示（壳只在 CI 的编译和界面测试里跑）。

## 关联

ADR-0007（Agent 与 Bot）、ADR-0015（标准接口）、ADR-0016（MCP 总线与审计）、ADR-0035（ACP 桥，已取代）、
ADR-0082（状态消息）、ADR-0108（单一 Hermes 内核）、ADR-0117（锁屏进度）。
