# ADR-0111 核心重启期间插件 Bot 的 Matrix 请求答可重试的 503，同步停了如实标异常

- 状态：已接受（2026-11-20）

## 背景

核心一重启（升级、崩溃、`docker restart`），Hermes 网关的 Bot 就再也收不到消息，而且一直报健康，只有重启网关容器才好
（ho-hermes 冒烟 11-20，卡 ho-core-sync-restart）。

原因在两侧：

- 核心先开监听、后恢复已装插件（`serve::run`）。网关一跳起来，插件的同步请求就到了，这时插件还没恢复，Tuwunel 也常常还没就绪。
- 网关的 `bot_request` 把“问不出这个令牌是谁”的三档揉成一个 403：核心正在恢复、问不到 Tuwunel（连不上、5xx）、令牌确实被吊销。
- 同一段时间里 Bot 的 MCP 请求（`POST mcp.wonton.internal/mcp/<令牌>`）也因为来源没登记回 403，上游把它归进永久错误
  （`mcp_tool_errors.py` 的 `_classify_mcp_failure`：`status_code in (401, 403)`），落进 auth park。即使 park 后还有每
  300 秒一次的自探（`mcp_tool_server_run.py` 的 `_park`），同步接上了工具也还要等这一趟，主人看到的就是「能聊天、工具一直不能用」。
- 上游 Hermes 的 `_sync_loop` 把 401/403 当永久错误（`_is_permanent_matrix_auth_error`），打一行
  `permanent auth error, stopping sync` 就 `return`，进程不退出，也不调 `_mark_disconnected`；`/health/detailed`
  只看平台状态字段，于是一直报 connected。

也就是说，403 让适配器“正确地”停掉了一个其实马上就能好的同步。

## 决策

1. `bot_request` 只把 whoami 的 401、403（令牌确实不认、账号停用）当永久的 403；核心还在恢复、来源还没登记、
   或问不到 Tuwunel 时答 503 `M_UNKNOWN`（`restoring`、`unavailable`）并带 `Retry-After: 5`。上游重试的间隔是写死的
   5 秒（`adapter.py` 的 `asyncio.sleep(5)`），不读这个头，值取 5 只是为了别的客户端，适配器不用改。
2. MCP 入口同一道门、同一个回答：来源还没登记又在恢复期间，答 503 `restoring` + `Retry-After: 5`，只有真的不认识的来源才答 403。
3. 网关把成功的同步请求记进 `PluginManager`；插件的健康检查除了 Bot 设备和容器健康，还看“还经核心同步吗”，
   超过 120 秒（长轮询最长 30 秒的 4 倍）没有一次成功同步就标 `Degraded` 并写明原因（四种语言的
   `plugin-settings-bot-sync-stalled`）。

不改上游适配器（[ADR-0108](0108-hermes-default-kernel.md)）：`/health/detailed` 的 connected 语义留给上游。
MCP 这条也不用改插件：503 是 transient，走上游自己的重试梯队（`mcp_tool.py` 的 `_MAX_RECONNECT_RETRIES = 5`、退避
1 秒起翻倍，上限 60 秒），梯队用完 park 之后 `_park` 的 300 秒自探会再起来，最迟 5 分钟自己接回；`/mcp refresh` 或
一次重连请求会立刻叫醒它。

## 理由

- 三种情况对适配器的意义不同：恢复中、问不到都可以马上再来，只有吊销重试也没用。用状态码分开是 HTTP 的原义。
- 上游自己判错的那一档（同步循环停了还报 connected），核心看得见：所有 Bot 的同步都经过核心。核心补一条自己的判断，
  比去猜上游的状态文件可靠，也不需要改上游。
- 120 秒留足了长轮询的等待时间和一次网络抖动；从没同步过的插件不报（不用长轮询的 Bot 不该被误伤）。

## 代价与待验证项

- 停下的是整轮重建检查，不是插件本身：状态是 `Degraded` 之后，下一轮巡检（30 秒）会重新跑一整轮对账，同步一恢复、
  检查通过就把状态写回 `Running`，最迟约半分钟回绿，不用主人动手；急着用就重启应用。
- 120 秒对 `sync` 超时较长的插件会误报；上游约定是 30 秒。
- 上一条只覆盖 Bot 经核心的同步；Agent 走别的收消息方式时不受这条保护。
- MCP 的 park 自探间隔是上游写死的 300 秒：核心恢复慢于重试梯队（约 30 秒）时，工具最迟 5 分钟后才回来，这期间
  状态同样是 `Degraded`。要更快得让插件在同步恢复后主动请求重连（上游的 `_signal_reconnect_and_wait`），留待冒烟观察。
- 这条检查只看得见“活过、后来停了”的同步：核心重启前同步就已经死掉的，重启后没有记录，这里报不出来。
