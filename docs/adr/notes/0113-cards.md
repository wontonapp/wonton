# ADR-0113 实现卡（前半：第 1–3 批）

按 ADR-0113 拆卡，顺序是“接口 → 服务器核 → 客户端核 → 壳 → 插件与验收”。每张卡先写会失败的行为测试再实现，由非作者 DeepSeek 复审。大小：S 半天内、M 一到两天、L 三到五天、XL 一周以上。

编号 M 开头（media），与其他线的卡号不冲突。第 1 批只改接口、不改行为，单独一个 PR，按 AGENTS.md 先合。第 2、3 批各自内部可并行，批间有依赖的标在“依赖”列。壳卡与验收卡见后半。

`_meta` 键、事件名、MCP 工具名一律以 ADR-0113 第 5 节与附录 `notes/adr0113-interfaces.md` 为准。

## 第 1 批：接口 PR（只加不改，单独 PR 先合）

| 编号 | 目标 | 改哪 | 依赖 | 大小 | 先红后绿的行为测试 | 并行 |
| --- | --- | --- | --- | --- | --- | --- |
| M1 | `mkdir wonton-protocol/src/media/`：`MEDIA_CAPABILITY`、`WONTON_USER_HEADER`、提供方路径常量、失败码字面量 | `crates/wonton-protocol/src/media/mod.rs`、`lib.rs` 加 `pub mod media;` | — | S | 单测断言常量值不被误改、五条路径拼接结果 | 可并行（M1–M4 同一 PR，按文件分） |
| M2 | 定义 `MediaKind`/`MediaOperation`/`MediaItemRef`/`DeviceProfile`/`PlaybackPlan` | 同 `media/mod.rs`，`serde` derive | M1 | M | 序列化往返；未知 `kind` 反序列化不 panic（AGENTS.md 向前兼容） | 同 M1 |
| M3 | 定义 `Task`/`TaskState`/`TaskKind`/`MediaFailure`/`MediaFailureCode`，状态机转移用 `TryFrom` | `media/task.rs` | M1 | M | 合法转移全过、非法转移（终态→`Running`）报错；表里标不退避的每一类都不进退避分支 | 与 M2 并行，靠 M1 |
| M4 | 定义 `Queue{revision}`/`PlaybackSession`/`MediaViewModel`/`PlaybackViewModel`/`MediaAction` | `media/session.rs`、`media/view.rs` | M1、M3 | M | `Queue` 的 `revision` 单调；`MediaAction` 未知变体不 panic | 与 M2 并行 |
| M5 | 能力表加 `media_source`（`Credentials::Required`，非高权限），清单 `provides.media_source` 结构 | `crates/wonton-core/src/plugin/capabilities.rs`、`crates/wonton-protocol/src/manifest/provides.rs` | M1 | S | 能力表快照断言存在与凭据策略；清单反序列化缺字段报错 | 与 M2–M4 并行 |
| M6 | `plugin-spec.md` 补 `media_source` 契约、错误信封、`provides.capabilities.media_source` 字段与示例 | `wonton-docs/plugin-spec.md`（**另一个仓、另一个 PR**） | M1–M5 | M | `plugin check` 用规范里的样例清单能过 | 不并行（要等接口类型定稿） |

## 第 2 批：服务器核（任务与会话的事实源）

| 编号 | 目标 | 改哪 | 依赖 | 大小 | 先红后绿的行为测试 | 并行 |
| --- | --- | --- | --- | --- | --- | --- |
| M7 | `media` 存储层：任务/会话/队列/配额账本落 SQLite，按人分 | `crates/wonton-core/src/media/store.rs` | M1–M4 | L | 建任务→读回→按人隔离（B 读不到 A 的）；配额超限拒写 | 与 M8 并行 |
| M8 | 任务状态机服务：合法转移、退避计时、终态不可逆 | `media/task_service.rs` | M3、M7 | L | 构造网络错→断言 `WaitingNetwork` 与退避序列；七次后 `Failed{network}`；手动重试清计数 | 不与 M7 并行同一文件 |
| M9 | 播放会话服务：一人一活动会话、接管发 `media.session.taken_over`、位置节流 | `media/session_service.rs` | M4、M7 | L | 第二设备 `play` → 旧设备收事件、新设备成活动；5 秒三次上报只转出一次 | 与 M8 并行 |
| M10 | 队列表服务：所有操作带 `revision`，不等回 409 带快照 | `media/queue_service.rs` | M4、M9 | M | 两边读同一 `revision` 后先写一方，后写方得 409 与当前快照 | 靠 M9 |
| M11 | 提供方客户端与凭据：按 `X-Wonton-User` 取凭据、转发、失败封解成 `MediaFailure` | `media/provider.rs`、`plugin/capabilities.rs` 的客户端 | M5、M7 | L | 假提供方回 401 → 任务 `Failed{authentication_required}` 不退避；缺凭据报 `permission_denied` | 靠 M7 |
| M12 | 契约测试：两个假提供方（只上传 / 只播放下载）跑同一组用例 | `plugin/tests/media_source_contract/mod.rs` | M11 | L | 同一组用例两边断言一致：重试次数、断点续、换提供方壳不改 | 靠 M11 |
| M13 | 设备命令通道：`GET /api/media/commands?device=&after=` 长轮询、幂等、`seq` 回执、30 秒超时报 `device_no_response` | `gateway/media.rs`、`media/commands.rs` | M7 | L | 挂起→投递→回执走通；30 秒无回执→工具结果带 `device_no_response` 列全起因；客户端核连着就长轮询取命令、逐条回执，做不了的命令（传输、备份、删离线副本、认不得的）立即回 `unsupported_by_device` 失败回执，不等核心超时 | 与 M7–M12 并行 |
| M14 | MCP 工具八个（`wonton__media_*`、`wonton__backup_now`），只读不发卡、变更发卡按 ADR-0024 | `mcp/media.rs`、工具注册 | M8、M9、M13 | L | 只读工具不发卡；`backup_now` 发本人卡；删离线文件走删除类卡 | 靠 M13 |
| M15 | 事件发出：`media.task.*`/`media.playback.*`/`media.session.taken_over` 进总线 | `media/events.rs` | M8、M9 | M | 汇聚完成→发 `media.task.done`；接管→发 `taken_over` | 靠 M8、M9 |
| M16 | 配额与清理：两本账本（提供方上传、设备离线），超限在 `Preparing` 拒绝 | `media/quota.rs` | M7 | M | 超上限拒建任务并回当前用量与上限；显式清理后额度回 | 与 M7 并行 |

## 第 3 批：客户端核（执行层）

| 编号 | 目标 | 改哪 | 依赖 | 大小 | 先红后绿的行为测试 | 并行 |
| --- | --- | --- | --- | --- | --- | --- |
| M17 | `Action::Media`、`subscribe_media`/`subscribe_playback`、`player_state`；任务/会话本地服务以服务器为准对齐 | `crates/wonton-client/src/media/mod.rs`、`action.rs` | M1–M4 | L | 重连后本地偏离的偏移被服务器值覆盖；退订后不再收片 | 与 M18 并行 |
| M18 | 上传执行：分片、`HEAD` 查偏移、偏移不符重开上传 | `client/media/upload.rs` | M17、M46 | L | 中途断网→重连从偏移续；偏移对不上→新 `upload_id` 从头；收到 `backup` 命令照备份设置建上传任务，回执带 `task` | 与 M17 并行不同文件 |
| M19 | 下载执行：`.part` 加 `Range`/`If-Range`，206 缺失或 ETag 变则从 0 | `client/media/download.rs` | M17、M46 | L | 断后续传不重下已得字节；ETag 变则丢弃 `.part` 从头；收到 `delete_offline` 命令删副本后 `PUT /offline` | 与 M18 并行 |
| M20 | 播放执行：把 `PlaybackViewModel` 翻成播放器回调与 `MediaAction`；5 秒及 pause/seek/stop 上报 | `client/media/player_bridge.rs` | M17 | L | 播放位置推进→上报节流；`taken_over`→停声并报最后位置 | 靠 M17 |
| M21 | `ViewModel` 翻成视图模型：任务进度、状态文案、失败原因带重试按钮 | `client/media/view.rs` | M17 | M | 六类不退避错误各出各自文案与正确按钮（无重试/带重试） | 与 M17 并行 |
| M22 | 页面媒体会话收口为 renderer：Android 的 `PageMediaOwner` 与 iOS 的 `PageMediaActive` 改成由播放服务统一持有，网页只报“在放什么”；Android 的 `PageMediaService` 被 M33 吸收 | `client/pages/media/mod.rs`、`PageMediaOwner.kt`、`PageMediaActive.swift` | M17、M20、M33 | L | 第三方页的 `navigator.mediaSession` 不被抢；页面关掉会话不死 | 靠 M20 |
| M23 | 请求头 `X-Wonton-Device` 的写入（说话人 `speakers` 归 0114，不在这张卡） | 客户端请求层 | M1–M4 | S | 每个请求都带；标识稳定不随重启变 | 与 M17 并行 |

第 3 批另有四张（字节路由在服务器核、依赖 M11；回环、Profile 在客户端核；桥要等两壳的播放器）：

| 编号 | 目标 | 改哪 | 依赖 | 大小 | 先红后绿的行为测试 | 并行 |
| --- | --- | --- | --- | --- | --- | --- |
| M43 | 媒体回环：照 `files/stream.rs` 起本机随机令牌回环，播放器与上传下载只连它 | `client/media/loopback.rs` | M17、M46 | L | 无令牌 401；`Range` 转发后被 `If-Range` 成 206；关闭后端口释放 | 与 M17 并行 |
| M46 | 服务器字节路由：上传建立/续写、取内容、取流三条路径、四个方法（见接口清单），`{item}` 是 `item_id`，`upload_id` 映射只在核心 | `crates/wonton-core/src/gateway/media.rs` | M11 | L | POST 由核心补体并记映射；PATCH 按映射续写；偏移 409 带当前偏移；`Range` 回 206；回环令牌过期 401（核心下发的一次性投屏地址另测） | 排在 M11 之后；与 M13 同文件，先说好先后 |
| M44 | `DeviceProfile` 组装：从壳上报的编解码支持拼出来，按人按设备缓存 | `client/media/profile.rs` | M17 | S | 缺能力时不发空数组；换设备各自的 Profile 不串 | 与 M17 并行 |
| M45 | 插件页“播这条”桥：`ui/initialize` 报 `media.play` 能力位、页面 RPC 拉起宿主播放器，成功回 `started` 并让页面 `pause()` | `pages/bridge`、`pages/media`、两壳 | M6、M30、M31 | L | 老壳不报能力位时页面不显示按钮；回包失败页面继续自己放 | 靠 M30/M31 |

并行小结：第 1 批（主仓）一个 PR 全进，M6 单独一个 wonton-docs PR。第 2 批 M7 先合，之后 M8 M9 M13 M16 可同时，M10 M11 M12 M14 M15 再上。第 3 批 M17 M46 M21 M23 M43 M44 可同时，M18 M19 靠 M46，M20 靠 M17 且与 M43/M44 一起，M22、M45 靠 M20 与 M33。所有壳卡等 M14 合入后再开，见后半。

## 第 4 批：两端壳（iOS、Android）

所有壳卡等 M14 合入后开。iOS 与 Android 同编号并排、两人各守一端，互不阻塞。界面卡附截图：默认深色与浅色各一张，尺寸 390×844（iPhone 14 逻辑点，Android 同宽高）；播放类卡另附锁屏与通知栏一张。

| 编号 | 目标 | 改哪 | 依赖 | 大小 | 先红后绿的行为测试 | 截图或并行 |
| --- | --- | --- | --- | --- | --- | --- |
| M24 | iOS 上传执行：`PHPhotoLibrary` 取增量、`URLSession` 后台会话按 M18 的分片上传 | `apps/ios/Wonton/Media/Upload.swift`、`project.yml` 加 `fetch`/`processing` | M18、M23 | L | 注入假 `URLSession` 走后台完成回调；杀进程重启后任务从服务器偏移续 | 与 M25 并行 |
| M25 | Android 上传执行：`MediaStore` 取增量、`WorkManager` + `dataSync` 前台服务按 M18 分片 | `apps/compose/androidApp/.../media/UploadWorker.kt`、`AndroidManifest.xml` 加 FGS 类型 | M18、M23 | L | 断言 FGS 在 `onTimeout` 中止并落 `WaitingPower`；重启从偏移续 | 与 M24 并行 |
| M26 | iOS 下载执行：`URLSession` 后台下载到 `.part`，完成后改名，`Range` 续 | `apps/ios/Wonton/Media/Download.swift` | M19、M23 | M | 断网重连不重下已得字节；ETag 变则丢 `.part` 从头 | 与 M25 并行 |
| M27 | Android 下载执行：`WorkManager` + `Range` 续到 `.part` | `.../media/DownloadWorker.kt` | M19、M23 | M | 同上；系统配额到→`Paused{reason=suspended_by_system}` | 与 M26 并行 |
| M28 | 备份设置页：开关、只在充电+Wi-Fi、选相册、当前用量与上限 | `apps/ios/Wonton/Media/BackupSettings.swift`、`.../media/BackupSettings.kt` | M24、M25、M16 | M | 关开关→任务转 `Paused{user}`；改条件立即生效；超限显示用量与清理入口 | 截图：设置页深/浅，含超限态 |
| M29 | 离线下载页：列表、进度、暂停/继续、删除、占用合计 | 两端 `Media/OfflinePage.*` | M16、M26、M27 | M | 删一项→配额回；`storage_full` 出清理入口；`unsupported_by_device` 出转码按钮 | 截图：列表页、失败行、删除确认，深/浅 |
| M30 | iOS 原生播放：`AVPlayer`、`AVAudioSession`、`PlaybackViewModel` 驱动、seek 范围 | `apps/ios/Wonton/Media/Player.swift` | M20、M22 | XL | 注入假播放器断言位置上报节流与 `taken_over` 停声报位 | 与 M31 并行 |
| M31 | Android 原生播放：Media3 `ExoPlayer` | `.../media/Player.kt` | M20、M22 | XL | 同上 | 与 M30 并行 |
| M32 | iOS 锁屏与耳机：`MPNowPlayingInfoCenter` + `MPRemoteCommandCenter`，按键翻 `MediaAction` | `apps/ios/Wonton/Media/NowPlaying.swift` | M30 | M | 按键回调只发 `MediaAction`，不本地改状态 | 截图：锁屏、控制中心 |
| M33 | Android 锁屏与通知栏：`MediaSessionService` | `.../media/PlaybackService.kt`、`Manifest.xml` 注册 | M31 | M | 服务存活时 `taken_over` 即清通知 | 截图：通知栏、锁屏 |
| M34 | 正在播放页：标题、进度、队列、上一首/下一首、投送 | 两端 `Media/NowPlaying.*` | M30/M31、M10 | L | 队列改带 `revision`；409 时重拉再重放 | 截图：页、队列抽屉、409 重试，深/浅 |
| M35 | PiP 两端：进出不停会话 | iOS `Player.swift`、Android `Player.kt` | M30、M31 | M（各一 L） | 进 PiP→会话仍 `Running`；出 PiP 位置不回跳 | 截图：PiP 态 |
| M36 | AirPlay：壳把播放器指向核心签的流地址（第一轮仅系统能力） | `apps/ios/Wonton/Media/AirPlay.swift` | M30 | M | 切 AirPlay→本地静音、位置仍上报；断连回本地 | 截图：AirPlay 选单 |
| M37 | 界面卡复审串：所有界面卡附深色截图，四语言 | 各界面卡 PR | M28、M29、M34 | S | 截图齐、String Catalog 四语 | 并行收尾 |

## 第 5 批：插件适配器（不依赖核心发版）

每张都在插件仓内、只靠 M1–M6 的契约；`media_source` 能力上线后即写，不等 M7 以后的服务端核。字段映射、错误码归一、上游凭据注入；不做重试、队列、状态。

| 编号 | 目标 | 改哪 | 依赖 | 大小 | 先红后绿的行为测试 | 截图或并行 |
| --- | --- | --- | --- | --- | --- | --- |
| M38 | Immich 适配：`upload` 翻 `POST /assets` + `bulk-upload-check`，`HEAD/PATCH` 对兜底分片 | `wonton-plugin-immich` 新 `media_adapter` | M1–M6 | L | 假 Immich 回 409 偏移→适配器回 409 带偏移；401→`authentication_required` | 可与 M39/M40 并行 |
| M39 | Subsonic/OpenSubsonic 适配：`playback` 翻 `stream`/`scrobble`/`savePlayQueue`，`download` 翻 `Range` | `wonton-plugin-...`（音乐插件仓）`media_adapter` | M1–M6 | L | `savePlayQueue` 失败不改会话态；`Range` 不支持回 206 或完整 | 可并行 |
| M40 | Jellyfin 适配：`playback` 翻 `PlaybackInfo`/`Session/Playing*`/`MediaSegments`，`list` 翻查库 | `wonton-plugin-jellyfin`（已有） | M1–M6 | L | 假 Jellyfin 回过期 `PlaybackPlan`→`media_gone` 不退避 | 可并行 |
| M41 | 三适配器的契约测试合入核：两个假提供方各以适配器为壳跑 M12 同一组用例 | `plugin/tests/media_source_contract/` | M12、M38、M39、M40 | L | 同一用例三适配器结果一致：重试次数、断点续 | 靠 M38–M40 |
| M42 | 端到端验收：真机从相册备份 20 张→在核看到进度→断网续→播一首、锁屏控→下 1 集、开飞行模式续→Agent 调 `wonton__media_status` 读得到 | 验收脚本 + `notes/adr0113-verify.md` | M14、M24–M36、M41、M45 | L | 每步断言事件与 ViewModel；失败项入“待验证” | 串行，最后一张 |

## 关键路径与并行分组

关键路径：M1–M6（接口）→ M7–M16（服务端核）→ M17–M23（客户端核）→ M24–M37（壳）→ M42（验收）。三适配器 M38–M40 与 M24–M37 同时开、不等服务端核。

可同时开工的组：

| 组 | 卡 | 约束 |
| --- | --- | --- |
| A 服务端 | M7 先，然后 M8、M9、M13、M16 | 同批内不同文件；M10、M11、M12、M14、M15 再上 |
| B 客户端核 | M17、M18、M19、M21、M23 | 不碰同一文件；M20 靠 M17，M22 靠 M20 |
| C 适配器 | M38、M39、M40 | 各在一仓，互不依赖；M41 在 C 全完开 |
| D 壳 | M24–M27、M28–M29、M30–M31、M32–M33、M35–M36 | iOS 与 Android 同题并排；界面类 M28、M29、M34 待 M21 |

## 待定

1. 大库首次备份在 Android 上跨 `dataSync` 六小时配额怎么分批，M25 实现时按实测定批大小。
2. M34 的队列抽屉是否与全屏播放页合并，待两壳界面原型复审后定。
3. M42 需真机与公网入口，开发机无凭据，验收记录写 `notes/` 不推。