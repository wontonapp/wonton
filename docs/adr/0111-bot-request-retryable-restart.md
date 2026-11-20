# ADR-0111 核心重启期间插件 Bot 的 Matrix 请求答可重试的 503，同步停了如实标异常

- 状态：已接受（2026-11-20）

## 背景

核心一重启（升级、崩溃、`docker restart`），Hermes 网关的 Bot 就再也收不到消息，而且一直报健康，只有重启网关容器才好
（ho-hermes 冒烟 11-20，卡 ho-core-sync-restart）。

原因在两侧：

- 核心先开监听、后恢复已装插件（`serve::run`）。网关一跳起来，插件的同步请求就到了，这时插件还没恢复，Tuwunel 也常常还没就绪。
- 网关的 `bot_request` 把“问不出这个令牌是谁”的三档揉成一个 403：核心正在恢复、问不到 Tuwunel（连不上、5xx）、令牌确实被吊销。
- 上游 Hermes 的 `_sync_loop` 把 401/403 当永久错误（`_is_permanent_matrix_auth_error`），打一行
  `permanent auth error, stopping sync` 就 `return`，进程不退出，也不调 `_mark_disconnected`；`/health/detailed`
  只看平台状态字段，于是一直报 connected。

也就是说，403 让适配器“正确地”停掉了一个其实马上就能好的同步。

## 决策

1. `bot_request` 只把 whoami 的 401（令牌确实不认）当永久的 403；核心还在恢复、或问不到 Tuwunel 时答 503 `M_UNKNOWN`
   （`restoring`、`unavailable`）并带 `Retry-After: 5`。上游对 503 和 5xx 一样，5 秒后重试，适配器不用改。
2. 网关把成功的同步请求记进 `PluginManager`；插件的健康检查除了 Bot 设备和容器健康，还看“还经核心同步吗”，
   超过 120 秒（长轮询最长 30 秒的 4 倍）没有一次成功同步就标 `Degraded` 并写明原因（四种语言的
   `plugin-settings-bot-sync-stalled`）。

不改上游适配器（[ADR-0108](0108-hermes-default-kernel.md)）：`/health/detailed` 的 connected 语义留给上游。

## 理由

- 三种情况对适配器的意义不同：恢复中、问不到都可以马上再来，只有吊销重试也没用。用状态码分开是 HTTP 的原义。
- 上游自己判错的那一档（同步循环停了还报 connected），核心看得见：所有 Bot 的同步都经过核心。核心补一条自己的判断，
  比去猜上游的状态文件可靠，也不需要改上游。
- 120 秒留足了长轮询的等待时间和一次网络抖动；从没同步过的插件不报（不用长轮询的 Bot 不该被误伤）。

## 代价与待验证项

- `Degraded` 之后插件退出容器巡检（和现有健康检查失败同一条路），同步自己好起来也不会自动回绿，要主人重启应用——
  文案就是这么说的。要不要自动重启网关容器，等有真实案例再看。
- 120 秒对 `sync` 超时较长的插件会误报；上游约定是 30 秒。
- 上一条只覆盖 Bot 经核心的同步；Agent 走别的收消息方式时不受这条保护。
