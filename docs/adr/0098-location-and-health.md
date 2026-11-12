# ADR-0098 手机的位置和健康数据接给服务器：按需查加手机主动同步

- 状态：已接受（2026-11-12）。“位置和健康走按需查加手机主动同步，按类型逐项授权，默认最小”由主人定（2026-11-12）；
  撤回后旧数据、保存期限、数据给谁、健康给云端模型、Android 位置发行、Android 健康、LPSE 申请七项同日定下（见第 3、7 节）
- 关联：[ADR-0003](0003-matrix-and-encrypted-push.md)（推送）、[ADR-0015](0015-small-components-standard-interfaces.md)（标准接口）、
  [ADR-0024](0024-confirmation-scope.md)（确认）、[ADR-0029](0029-platform-material.md)（平台）、[ADR-0043](0043-app-languages.md)（语言）、
  [ADR-0053](0053-plugin-dependencies.md)（能力）、[ADR-0066](0066-plugin-forms.md)（插件形态）、[ADR-0088](0088-family-accounts.md)（家人账号）

## 背景

用户想让家里的 Agent 回答“我现在在哪”“到家了提醒我”“这周睡得怎么样”。数据在手机上，Agent 在家里的服务器上。
手机系统只让前台或被系统唤醒的 App 取这些数据，取得到的时机和次数都由系统定。

外部事实（2026-11-12 抓取 Apple、Google 现行文档，iOS 以 iOS 26 系文档为基线，iOS 27 有无变化未核实）：

| 事实 | 出处 |
| --- | --- |
| 静默推送（`content-available`）低优先级、不保证送达，Apple 建议每小时不超过两三条，App 被用户划掉后收不到，唤醒后约 30 秒 | [Pushing background updates](https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app) |
| 位置推送扩展（LPSE，iOS 15+）App 没运行也能被 `apns-push-type: location` 唤醒取一次位置；要“始终”授权和 `com.apple.developer.location.push` 授权项；每台设备约 360 次/24 小时、每几分钟回补一次，用完系统会退订一段时间，查不到余量；**只给“让人把位置分享给他明确同意的人”和急救类 App**；离开设备的位置数据建议端到端加密 | [Creating a location push service extension](https://developer.apple.com/documentation/corelocation/creating-a-location-push-service-extension) |
| 显著位置变化：移动约 500 米以上、不早于 5 分钟一条；App 被杀后系统会在后台拉起；第一条通常是缓存的旧位置，要看时间戳 | [startMonitoringSignificantLocationChanges()](https://developer.apple.com/documentation/corelocation/cllocationmanager/startmonitoringsignificantlocationchanges()) |
| HealthKit 按数据类型逐项授权；**App 看不出用户拒绝了读取**，拒绝时就像没有数据；手机锁着时读不到（写入会缓存）；只能用于健康健身用途并在界面和宣传里写清；未经明确许可不得给第三方，且对方也须给用户提供健康服务；必须有隐私政策 | [Protecting user privacy](https://developer.apple.com/documentation/healthkit/protecting-user-privacy)、[authorizationStatus(for:)](https://developer.apple.com/documentation/healthkit/hkhealthstore/authorizationstatus(for:)) |
| HealthKit 后台投递要 `com.apple.developer.healthkit.background-delivery`，多数类型最快每小时一次，连续三次不回应就停 | [enableBackgroundDelivery](https://developer.apple.com/documentation/healthkit/hkhealthstore/enablebackgrounddelivery(for:frequency:withcompletion:)) |
| App Review 5.1.3(i)：健康数据不得用于广告、营销、数据挖掘，不得给第三方（健康研究经许可除外），须写明收集哪些健康数据；(ii) 不得把个人健康信息存进 iCloud | [App Review Guidelines 5.1.3](https://developer.apple.com/app-store/review/guidelines/) |
| Google Play 后台位置：须是核心功能且对用户有明显好处；Play Console 申报（全 App 只能报一个功能）、30 秒内视频、使用中醒目披露（含“位置”“App 关闭时”等字样，在系统弹窗之前）、隐私政策专门写位置 | [Play 后台位置政策](https://support.google.com/googleplay/android-developer/answer/9799150) |
| Health Connect 后台读要 `READ_HEALTH_DATA_IN_BACKGROUND`；默认只能读授权前 30 天，更早的要 `READ_HEALTH_DATA_HISTORY`；Play 只批健康、健身、医疗用途，有专项审核 | [Health Connect 读数据](https://developer.android.com/health-and-fitness/guides/health-connect/develop/read-data)、[Play 敏感权限政策](https://support.google.com/googleplay/android-developer/answer/9888170) |

先例：Home Assistant Companion 用显著位置变化加区域上报位置，服务器发推送命令 `request_location_update` 让手机补报一次，后台频率由 iOS 定
（[位置](https://companion.home-assistant.io/docs/core/location)、[推送命令](https://companion.home-assistant.io/docs/notifications/notification-commands)）。
Health Auto Export 把 HealthKit 推到自建 REST，文档同样写明锁屏读不到、后台不保证执行
（[REST API](https://help.healthyapps.dev/en/health-auto-export/automations/rest-api)）。

## 决策

### 1. 两条路，按数据分工

| 数据 | 手机主动同步 | 按需查 | Agent 问“现在”时拿到的 |
| --- | --- | --- | --- |
| 位置 | 显著位置变化、进出主人设的区域、App 在前台时，各报一个点 | 核心唤醒手机取一个新的点 | 按需查到的新点；查不到就报原因，另外可以查历史里最后一个点（带测量时间） |
| 健康（每个类型） | 打开 App 时补齐；后台投递按类型最快每小时一批 | 只是“请手机同步一次”，不另开读数通道 | 服务器上已同步的数据，带这个类型同步到什么时候（`synced_through`） |

- 位置变化快、取一次的成本系统给得起，所以“现在在哪”按需查；历史和“到家提醒”这类事件靠同步。
- 健康数据锁屏读不到，按需查多半落在锁屏时，所以健康以同步为主；Agent 问“今天走了几步”读服务器上的数据，结果写明截至何时，
  Agent 自己决定要不要说“数据截至 9:40”。要更新时 Agent 可以请手机同步一次，失败照第 6 节报原因。
- 两条路互不替代：按需查失败不退回同步来的旧点冒充“现在”，同步断了也不拿按需查补历史。

### 2. 按需查怎么走

1. Agent 调核心 MCP 工具（第 4 节）。核心核对授权（第 3 节）和限流，记下一个请求编号。
2. 核心经推送代理（ADR-0003）给这个人开了该类型的设备发一条“数据请求”推送，推送里只有请求编号和类型，没有数据；同一个人的几台设备都发，取先回来的。
   - iOS 位置：优先 LPSE（`apns-push-type: location`），App 被划掉也能唤醒；没拿到授权项或用户只给了“使用期间”时，退回静默推送。
   - iOS 健康“同步一次”：静默推送。
   - Android：“保持连接”开着时经隧道直接下发；用 FCM 时发高优先级消息。
3. 手机被唤醒后由客户端核心经隧道把结果交回核心（同一请求编号），核心把结果交给这次工具调用。
4. 等待上限 30 秒（静默推送唤醒后的执行窗口），到时没回来就按第 6 节报“手机没回应”。

限流：每人每小时按需查位置默认最多 12 次（LPSE 的日额度折算后留一半余量），超过直接报 `rate_limited`，不发推送。
静默推送的系统限流核心看不到，所以第 6 节把“没回应”的可能原因一起写明，不猜是哪一个。

### 3. 授权：按类型、按 Bot、默认全关

- 类型清单由核心定，第一批：位置；健康里的步数、步行跑步距离、活动能量、心率、静息心率、睡眠、体重。都默认关。
  清单只增不改名，新版 App 不认识的类型不显示（AGENTS.md 的“向前兼容”）。
- **保存期限**：位置默认 30 天，健康默认 1 年；在“手机数据”页按类型可改，可选“不保存”。到期数据由存储提供方按保存期限清理。
- 每个人在 App 的“设置 → 手机数据”里逐项打开。打开时依次：本页披露（第 7 节）→ 系统权限弹窗（壳调用）→ 选哪些 Bot 能读（只列这个人的私人 Bot，默认勾他的默认 Agent）。
  位置再分两档：“只在打开 App 时”只要“使用期间”授权；“后台同步和随时查”才申请“始终”。
- **数据给谁**：数据属于手机的主人（数据主体）。只有他的私人 Bot、而且是他勾过的才能读；共享 Bot、别人的 Bot、管理员都读不到。
  Bot 在群聊里替别人干活时（ADR-0061 的 `X-Wonton-User` 不是数据主体）一律拒绝。家人之间互看位置是另一个功能，不在本篇。
- **读取不发确认卡片**：授权在打开时按类型、按 Bot 一次给定，之后这些工具按 ADR-0024 第 2 步的“不敏感”处理（核心工具里明确标为只读、不发卡片）。
  理由：Agent 的“到家提醒”“每天总结”都在无人值守时跑，每次一张卡片等于不能用；而授权的范围已经比卡片的“30 分钟内都允许”窄。
  每次读取（含按需查）都写审计，“手机数据”页按类型列出最近哪个 Bot 什么时候读过。
- **撤回**：在 App 里关掉一项立即生效：手机停止上报、丢掉没传的队列，核心随即拒绝该类型的读取。关掉时默认勾选“同时删除服务器上已存的这类数据”，可取消；
  在系统设置里关掉位置权限，壳能看到状态变化，报给核心，状态显示“系统里关了”，已存的数据不动。HealthKit 拒绝读取看不出来，只表现为没有新数据，所以健康类型显示
  “最后同步时间”，太久没有新数据时写“可能在系统里关掉了”，不判定为已撤回。
- 家人被移除、账号删除时，他的手机数据一起删。

### 4. 给 Agent 的接口：核心 MCP 工具

| 工具 | 做什么 | 结果 |
| --- | --- | --- |
| `wonton__location_now` | 按需查一次位置 | 纬度、经度、精度（米）、测量时间、来源设备 |
| `wonton__location_history` | 时间段内的点 | 点列表，每个点带测量时间 |
| `wonton__health_query` | 某类型在时间段内的样本或按天汇总 | 样本或汇总、`synced_through` |
| `wonton__health_refresh` | 请手机同步一次某类型 | 同步到了什么时候，或失败原因 |
| `wonton__personal_data_status` | 这个 Bot 能读哪些类型，各自最后同步时间、系统授权状态 | 状态列表 |

- 都作用于这个 Bot 的数据主体，参数里没有“谁”，免得 Agent 问到别人的。
- 用工具，不用 MCP 资源：资源要 Agent 主动订阅和读，各家 Agent 支持参差；按需查本来就是一次带副作用（唤醒手机）的调用。
- 没授权的类型照常出现在 `tools/list`，调用时报 `permission_denied`，让 Agent 能告诉用户去哪里打开。

### 5. 组件边界和存储

| 组件 | 做什么 |
| --- | --- |
| 壳（Swift/Kotlin） | 调 CoreLocation、LPSE、HealthKit、Fused Location、Health Connect；画系统权限之前的披露页；把读到的原始值交给客户端核心 |
| 客户端核心（Rust） | 各类型的开关和系统授权状态、待传队列（离线攒着、连上补传、去重）、按需请求的应答、`PersonalDataViewModel` |
| 服务器核心 | 设备和推送（已有）、按需请求的转发和限流、按人按类型按 Bot 的授权记录、第 4 节的 MCP 工具、审计；把收到的同步数据转给存储提供方，撤回、删除时叫提供方删 |
| 存储提供方（插件） | 存和查：能力 `personal_data`，一个官方系统插件提供，可替换 |

- 存储放插件，不放核心：核心按 AGENTS.md 只做插件管理、网关、推送、总线和授权；时间序列的存、查、清理和相册一样是一种能力。
  核心只当它的使用方，用法同 ADR-0066 第 3 节核心用 `remote_mcp`：按能力找提供方，不认某个插件。
- 授权只在核心判定：提供方不注册给 Agent 的 MCP，Agent 只经第 4 节的工具读，换提供方不会绕过授权。
- 能力 `personal_data` 的契约（进 ADR-0053 的能力表，HTTP 传输，凭据按使用方，不算高权限）：
  - 每个请求带 `X-Wonton-User`（数据主体），提供方按人分开存。
  - 写入：位置点用 OwnTracks 的 JSON（`_type: location`，[格式](https://owntracks.org/booklet/tech/json/)），这是 OwnTracks Recorder、Dawarich、Home Assistant
    都收的写法；健康样本用 `wonton-protocol` 定义的 JSON，类型名就是第 3 节清单里的名字，值带单位和起止时间。健康没有选 FHIR：只为存几种计数和睡眠段，
    FHIR 服务器（如 HAPI）太重，Observation 的写法也不比自己的简单。
  - 查询、按人按类型按时间段删除、按人导出、按保存期限清理。
  - 契约测试照 [`model_gateway_contract`](../../crates/wonton-core/src/plugin/tests/model_gateway_contract/)：两个假提供方跑同一组用例，断言核心的工具结果、
    撤回删除、导出在两边一样。
- 提供方：官方插件“手机数据”，SQLite 存。消费方：核心的第 4 节工具；之后的“到家提醒”这类自动化也经这些工具。
- 想把位置同时记进 Dawarich、Traccar 这类应用的，由“手机数据”插件按 OwnTracks HTTP 转发一份（插件设置里开），不在核心里做第二条路。

### 6. 失败即停：取不到就如实报

工具结果的 `_meta` 里 `app.wonton/failure` 给类别，文字给 Agent 看：

| 类别 | 什么时候 |
| --- | --- |
| `permission_denied` | 这类数据没打开，或没勾这个 Bot，或说话人不是数据主体 |
| `os_permission_missing` | 系统权限没给或被关了（位置能看出；健康看不出，不报这个） |
| `device_unreachable` | 这个人没有开了该类型的设备，或推送渠道没登记 |
| `device_no_response` | 推送发出去 30 秒没回应：可能手机关机、没网、被划掉或被系统限流，原文照列这几种，不挑一个 |
| `device_locked` | 健康：手机锁着读不到 |
| `rate_limited` | 核心的每小时上限到了，带下次可查的时间 |
| `not_supported` | 平台没有这种数据（如 iPad 没有 HealthKit、Android 版没做健康） |
| `provider_unavailable` | 存储提供方没装、没就绪或出错，带原始错误 |

- 按需查失败时不在同一结果里附带历史里的旧点；Agent 要旧点就另外调 `wonton__location_history`，拿到的点带测量时间。
- 手机交回的位置测量时间早于请求时刻 2 分钟以上（显著位置变化的第一条常是缓存），按 `device_no_response` 处理，不当新位置交出。
- 同步数据缺口不补造：哪段时间没同步到就是没有，`synced_through` 写到真实同步到的时刻。

### 7. 平台和审核

- **iOS 文案**：`NSLocationWhenInUseUsageDescription`、`NSLocationAlwaysAndWhenInUseUsageDescription`、`NSHealthShareUsageDescription` 写明“发到你自己家里的云吞服务器，
  给你允许的 Bot 用”。不申请 `NSHealthUpdateUsageDescription`：不往 HealthKit 写。文案进 String Catalog 四种语言（ADR-0043）。
- **5.1.3 怎么满足**：不做广告、不卖、不进 iCloud；披露页逐项列出收集的健康类型；数据只到用户自己的服务器。风险在“第三方”：Bot 读到的内容会交给 Bot 用的模型服务。
  披露页写明“Bot 读取后，内容会发给这个 Bot 使用的模型服务（显示服务名）”，打开健康类型时单独同意这一条。App 的介绍和截图写明健康功能（HealthKit 只能用于健康健身用途并写清）。
- **LPSE 资格**：Apple 只给“让人把位置分享给他明确同意的人”和急救类 App。申请时按“把位置分享给用户自己允许的家人和 Bot”报；拿不到时 iOS 按需查只走静默推送，
  能用但不可靠，第 6 节的 `device_no_response` 会更常见。
- **Android 位置**：后台同步和 FCM 唤醒取位置都要后台位置权限，上 Play 要走申报、视频和醒目披露。先只在直装 APK 里做（直装不受 Play 审核）；上 Play 时再补申报和披露。
- **Android 健康**：Health Connect 的 Play 专项审核只批健康、健身、医疗用途。先做 iOS；Android 健康先不做，健康类型在 App 里显示为 `not_supported`。

## 没选的做法

- **只做按需查**：权限少，但 iOS 静默推送不可靠、健康锁屏读不到，历史和“到家提醒”做不了。主人已否。
- **只做同步**：“现在在哪”只能给最多几分钟到几十分钟前的点，且静止时不再上报，Agent 无法区分“没动”和“断了”。
- **用第三方 App 当提供方（Home Assistant Companion、OwnTracks、Dawarich App、Health Auto Export）**：用户要多装一个 App、另配一套服务器地址和密钥，
  授权和撤回分散在几处，核心管不到谁读了。它们作为第 5 节存储能力的写入来源可以接（OwnTracks 写法本来就是契约），但不是默认路径。
- **存储放核心**：少一个插件，但违反“核心保持薄”，保存期限、导出、转发 Dawarich 这些会持续长在核心里。
- **每次读取发确认卡片**：见第 3 节，无人值守的场景全部不能用。
- **位置后台连续定位（`allowsBackgroundLocationUpdates`）**：iOS 有常驻指示条、耗电大，家用场景不需要米级轨迹。

## 代价与待验证项

- 新增一种推送（数据请求），中继要支持 `apns-push-type: location`；gorush 是否支持这个推送类型未核实。
- LPSE 授权项可能拿不到；拿不到时 iOS 的按需查质量明显下降。
- 健康数据经模型服务出家门，App Review 对“第三方”的口径未实测，可能被拒；被拒时按主人 11-12 定的退路，改为健康只给用本地模型的 Bot。
- “每人每小时 12 次”和“30 秒”两个常量是按文档定的，要在真机上测唤醒耗时和成功率再调。
- iOS 会定期提醒用户“某 App 在后台用了你的位置”，用户可能因此关掉“始终”；频率未核实。

## 实施顺序

1. `wonton-protocol`：类型清单、`personal_data` 能力契约、失败类别、`PersonalDataViewModel`（只改接口）。
2. 存储提供方插件“手机数据”和契约测试（两个假提供方）。
3. 服务器核心：授权记录、同步转发、第 4 节的历史和健康查询工具、撤回删除、审计。
4. 客户端核心加 iOS 壳：设置页、披露、位置同步（显著位置变化、区域）、健康同步（后台投递）。
5. 按需查：数据请求推送、静默推送唤醒、`wonton__location_now`、`wonton__health_refresh`、限流；LPSE 拿到授权项后单独一个 PR。
6. Android 壳的位置（先只在直装 APK，上 Play 时再申报）；Android 健康不做，显示 `not_supported`。
7. 端到端冒烟：真机打开位置，Bot 答出“我在哪”和“今天走了几步”，关掉后 Bot 报 `permission_denied`。
