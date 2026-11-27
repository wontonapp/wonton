# ADR-0113 媒体与传输：长任务由核心持有，插件经媒体源能力提供内容

- 状态：提议（2026-11-21），影音相册线起草，插件底座线复审后合并；2026-11-27 补充第 9 节（相册原生浏览、去重、实况照片）
- 取代 [ADR-0014](0014-reserved-capabilities.md) 的“Subsonic 原生音乐页”一节（主人 2026-11-21 定回原生）；补充 [ADR-0005](0005-native-clients-thick-core.md)（跨设备长任务的事实源在服务器核）
- 关联：[ADR-0005](0005-native-clients-thick-core.md)（厚核心薄壳）、[ADR-0015](0015-small-components-standard-interfaces.md)（小部件、标准接口）、
  [ADR-0016](0016-mcp-bus-and-confirmations.md)（MCP 总线与确认）、[ADR-0020](0020-file-service.md)（文件服务）、
  [ADR-0024](0024-confirmation-scope.md)（每次都问的三类）、[ADR-0032](0032-connection-quality.md)（连接恢复）、[ADR-0033](0033-plugin-tabs.md)（插件页面）、
  [ADR-0038](0038-multi-step-cards.md)（卡片）、[ADR-0045](0045-family-members.md)（家人）、[ADR-0088](0088-family-accounts.md)（角色与按人身份）、
  [ADR-0095](0095-no-tab-bar.md)（没有底栏）、[ADR-0098](0098-location-and-health.md)（个人数据能力、后台唤醒）、[ADR-0104](0104-view-file-media.md)（看文件、下载落点）
- 边界：[ADR-0114](0114-plugin-agent-platform.md) 管插件 Agent 的通用底座（家人账号、群聊说话人、事件唤醒、按人通知、卡片模板、深链、结果可见范围、文件交给插件）；
  本篇只管长任务（备份上传、播放会话、离线下载）和媒体源的契约，事件投递、卡片、深链只引用 0114 的项名，不重写
- 主人 11-21 定：备份上传、音乐和视频的播放会话（含队列）、离线下载这几样长任务由**核心**持有，按一套通用能力设计，不按相册、音乐、影视各写一份；
  两端壳只接系统能力并出声、出画（`brief/adr0113.md`）

## 背景

三件事的形状是同一个：一个家人在手机上让某件事跑很久（把 200 张照片传回家、放一张专辑、下一部片），换设备或锁屏都不能丢，Agent 要能问进度、能插手。
三块方案各自把它拆成了同一张“核心的后台任务”卡（`notes/summary.md` 的“三块共用的底座项”表最后一行），所以底座只出一份。
内容在插件那边，三家都没有 Wonton 定的接口：Immich 的上传接口、Subsonic/OpenSubsonic 的播和进度、Jellyfin 的 `PlaybackInfo`/`Sessions`。

### 现状

- 手机传输已经有了：上传走 tus（`crates/wonton-client/src/files/upload.rs`），下载走 `Range`/`If-Range` 加 `.part`（`download.rs`），
  给播放器的回环令牌有现成实现但只认聊天文件（`crates/wonton-client/src/files/stream.rs:1-8`），失败分类和传态在 `wonton-protocol/src/file/mod.rs:97-142,291-320`。
  它们是面向聊天文件的，不认库、不认播放会话。
- 插件网页的媒体会话已经有了（#585）：页内 `navigator.mediaSession` 上报快照，客户端核解析后交壳（`crates/wonton-client/src/pages/media/mod.rs:1-133`、
  `apps/compose/shared/src/androidMain/kotlin/app/wonton/media/PageMediaOwner.kt:42-53`）。会话跟着页面活，页面一死会话就死。
- 能力表里 `personal_data` 是现成样板：能力名核心定、提供方要对使用方签凭据、核心侧有客户端、两个假提供方跑契约测试（`crates/wonton-core/src/plugin/capabilities.rs:120-128`、
  `plugin/tests/personal_data_contract/mod.rs:1-10`）。

### 平台限制（外部事实）

| 事实 | 出处 |
| --- | --- |
| iOS 后台下载用 `URLSession` 后台会话，交给系统传；App 被挂起、被杀都能继续，完成后交回启动；不带后台会话的传输杀进程就断 | [Downloading files in the background](https://developer.apple.com/documentation/Foundation/downloading-files-in-the-background) |
| iOS 的后台模式要逐项声明；App 只有 `audio` 一项（`apps/ios/project.yml:69`），后台跑传输、跑一次处理任务要另加 `fetch`/`processing` | 同一文件；[UIBackgroundModes](https://developer.apple.com/documentation/bundleresources/information-property-list/ubackgroundmodes) |
| Android 前台服务 `dataSync` 一天总时长 6 小时（24 小时内），到点系统回调 `onTimeout`；前台服务要声明类型和权限 | [Foreground service timeouts](https://developer.android.com/develop/background-work/services/fgs/timeout) |
| Android 播放要 `MediaSessionService`（锁屏、耳机、车机都以它为准），通知由它自己出 | [MediaSessionService](https://developer.android.com/guide/topics/media/session/mediasessionservice) |
| CarPlay 要单独申请 CarPlay 授权项；Android Auto 要 `MediaLibraryService`（Media3） | [CarPlay entitlements](https://developer.apple.com/documentation/carplay/requesting-carplay-entitlements)、[Android for Cars](https://developer.android.com/training/cars/media) |

### 行业做法（只看文档和行为）

Plex 客户端只接自己的服务器；Synology 的客户端只接自家 NAS；Infuse 是单体，把 SMB/NFS/WebDAV/Emby/Jellyfin/Plex 都编进客户端。
三家都靠客户端内置协议客户端接多种源，没有插件生态可照搬；能接多少种源由发版决定。
出处：Plex 的官方客户端与服务器文档、Synology Photos 与 DS video 的支持来源页、Infuse 的支持文件系统列表（均为官方文档）。
许可证：Jellyfin 服务端 GPL-2.0、Navidrome GPL-3.0、Immich AGPL-3.0，都是独立容器，不链接进我们的二进制；本文与适配器只按它们的官方 API 文档与 OpenAPI 设计，不读也不抄它们的客户端源码（Navidrome 网页是 GPL-3、Jellyfin/Immich 的 App 各有自己的许可证）。适配器自己只引入 MIT/Apache 的库。

## 决策

### 1. “核心”按持久性拆：服务器核是事实源，客户端核只管执行

长任务和播放会话的状态放在**家里服务器的核心**（`crates/wonton-core`），一份；手机上的客户端核（`crates/wonton-client`）只执行：读手机上的字节、驱动原生播放器、按节奏上报进度、把状态翻成 ViewModel 交给壳。

| 谁 | 持有什么 | 不持有什么 |
| --- | --- | --- |
| 服务器核 | 任务与会话的编号、种类、目标、状态机、进度、重试计数与下次重试时刻、配额账本、人与设备的绑定、队列及其 `revision`；对提供方的凭据（按人取） | 手机上的字节、播放器的句柄、系统权限状态 |
| 客户端核 | 正在跑的任务的本地执行上下文：分片偏移、`.part` 文件、重试计时器、播放器回调到上报的转换、`MediaViewModel`/`PlaybackViewModel` | 任务的权威状态；本地状态在每次连上服务器后以服务器为准对齐 |
| 壳 | 系统能力：`URLSession` 后台会话、`BGTaskScheduler`、`WorkManager`、`AVPlayer`/`AVAudioSession`、`MPNowPlayingInfoCenter`/`MPRemoteCommandCenter`、Media3 `ExoPlayer`/`MediaSessionService`、`PHPhotoLibrary`、`MediaStore`；出声、出画 | 任何决策 |

理由：

- 要活过重装和换设备。任务和队列在服务器上，重装 App、换手机后重新配对就接上；放客户端核则卸载即失，跨设备队列没有落点（主人 11-21 定的“在客厅那台继续放”要求这个）。
- Agent 要看得见、插得上手。Hermes 在服务器上，读进度、暂停、改队列都直接打服务器核（第 5 节），不必等某台手机醒着。
- 字节不能多绕，但手机进不了插件网段。上传、播放、下载的字节都由手机上的客户端核经设备隧道发到核心网关，核心按能力别名转给提供方；**核心只中转、不缓冲**，不落盘也不解码。路径、每一跳的鉴权与头见第 2 节的“字节路径”。
- 不违反 ADR-0005：厚核心说的是“状态、逻辑、网络放客户端核，壳薄”，指的是单机内的分工；ADR-0005 没有说状态必须只活在单机上。服务器核本来就有插件管理、网关、推送、总线、授权（`AGENTS.md` 产品边界），长任务的跨设备事实源属于“总线与授权”这一侧。
- 先例是 ADR-0098 第 5 节：存储放插件，核心只是使用方，核心不因为“要存东西”就变厚。这里同理——媒体内容和字节都在提供方，服务器核只持状态。

没选的做法：

- **全放服务器核**：驱动不了壳。播放位置、系统权限、播放器生命周期只有客户端核和壳看得到；服务器核要驱动原生播放器就得每件事一次往返，锁屏下的 5 秒上报会变成一条长连接。
- **全放客户端核**：重装即失、Agent 看不到、跨设备没有落点，同一首歌在两台手机上各有一份队列，谁也不是“那个队列”。
- **放壳里**（Swift/Kotlin 各写一份）：两端各写一遍状态机，行为会分叉，且 Agent 还得另开一条路读。

### 2. 插件协议：核心定义 `media_source` 能力，插件出薄适配容器

核心定义一种能力 `media_source`，插件当**提供方**，核心当**使用方**。每个要接媒体的插件出一个薄适配容器，把上游接口翻成这套能力；**服务器核**不认识 Immich、Subsonic、Jellyfin 的名字（例外的读路径见下）。

**它就走 `provides.capabilities` 那张表，不开专用顶级字段。** 选路（`is_provided`/`is_self_provided`/`dependencies::service`）、凭据签发与校验、安装确认、`spec_floor` 全都复用现成机制，只在 `ProviderService` 上加两个可选数组 `operations`、`kinds`、并在现有 `credentials` 块里写 `issue`/`revoke` 命令。理由：加同名顶级字段会被当成“没提供”（`capabilities.rs` 的选路只读 `provides.capabilities`），且 `Credentials::Required` 在代码里的意思是“提供方必须声明凭据命令”（`capabilities.rs:52`、`provides.rs:67-73`），没有命令就发不出凭据。

- 形状照 `personal_data` 的现成样板（`crates/wonton-core/src/plugin/capabilities.rs:120-128`、`plugin/tests/personal_data_contract/mod.rs:1-10`）。契约测试两个假提供方——一个像相册（只能上传）、一个像点播（只能拉流）——证明核心对两者用同一套代码。
- `operations` 取值 `upload`（开始/查偏移/写分片/收尾/放弃）、`download`（带 `Range` 的取字节）、`playback`（要播放计划、报进度、报停止）、`list`（分页列条目，给 Agent 找内容）；`kinds` 取值 `photo`/`video`/`audio`。一个提供方可以只声明几样。
- 清单要 `interface: "http"`、`hosted: "plugin"`、`credentials: required` 和 `credentials` 的 `issue`/`revoke`，缺一个都是清单非法（`capabilities.rs:14-31`）。
- `provides.media_source.browse`（spec 0.17）声明客户端核心还能直接按哪个标准浏览库，取值 `opensubsonic`；认不得的落到 `Other`，客户端当没声明、照旧开网页。只能写在 `media_source` 上，插件还要有 `sso: proxy` 的 `webui`：客户端经网页的代理讲，靠网关代加的身份头认人（`plugin/manifest_browse.rs`）。`InstalledPlugin.media_browse` 把它带给客户端，点开插件进原生界面，网页放进胶囊的“更多”。核心只认这个字段、不认插件名（ADR-0015）。
- **和已有 `jellyfin` 能力并存，不合并。** `jellyfin` 是给 Seerr 这类“在 Jellyfin 里建请求/读库”的插件用的窄能力（`capabilities.rs:85-91`），`media_source` 是给播放与传输用的宽契约。同一个 Jellyfin 插件可以两样都提供，凭据各发各的；Seerr 不迁。写进规范：两者互不替代。
- **读路径的例外（音乐）。** OpenSubsonic 是开放标准，艺人、专辑、歌单、流派、搜索、歌词、收藏、评分、历史这类只读浏览由客户端核直讲，不经 `list`（`list` 只给 Agent 找内容用，字段少、翻不出原生页）。客户端核经插件网页的代理去讲（`crates/wonton-client/src/music_library/`），身份由网关按设备的人注入 `Remote-User`，客户端不存上游口令（Navidrome 0.64.2 实测：白名单来源带这个头时 `/rest` 不要 `u`/`p`，头里的人不存在回 40，带头打开一次 `/app/` 就建号）。请求层自己写、只盖用到的端点，没用 `opensubsonic` 库：它每个请求都强制带 `u`/`t` 或 `apiKey`，没有只靠代理头认人的模式。ADR-0015 第 4 条要求写明：只讲 OpenSubsonic 这一个标准，音源是标准接口、不随插件变；出声的流、scrobble、断点、队列仍走 `media_source`。
- 适配器里没有业务逻辑：Immich 的 `POST /assets` 加 `bulk-upload-check` 翻成 `upload`；Subsonic/OpenSubsonic 的 `stream`/`scrobble`/`savePlayQueue` 翻成 `playback` 与队列快照；Jellyfin 的 `PlaybackInfo`/`Sessions/Playing*`/`MediaSegments` 翻成 `playback` 与章节。适配器只做字段映射、错误码归一、上游凭据注入（第 6 节），不做重试、不做队列、不存状态。
- 上游凭据由适配器自己从插件数据卷取，核心只给“这是谁”（`X-Wonton-User` 头）。核心不存上游密码，只存“哪个人对应提供方的哪把能力凭据”。

**字节路径（每一跳都写死）。** 能力别名只在插件网段里（`plugin-spec.md:1625-1627`），手机连不到，所以：

1. 客户端核在本机起回环 HTTP（照 `files/stream.rs:1-8` 的随机令牌），播放器、上传器只连 `127.0.0.1`。
2. 客户端核把请求经设备隧道发给核心网关 `/_wonton/api/media/…`，凭证是设备隧道身份（ADR-0004、ADR-0009）；`X-Wonton-User` 由核心按设备归属的人填，能力凭据由核心从提供方签的那把取，两者都不进壳。
3. 核心按能力别名转发给提供方，字节**流式透传、不落盘**；`Range`、`If-Range`、`Content-Range` 原样过。客户端只认三条核心路由，路径里带任务/会话和 item，核心按它查到提供方：
   `POST|PATCH /_wonton/api/media/tasks/{id}/uploads/{item}`（`{item}` 是 `item_id`；POST 无体、核心用任务里的 `UploadItem` 补体建上传并记住 `upload_id` 映射，PATCH 按映射续写，两者都回当前 `upload_offset`）、
   `GET /_wonton/api/media/tasks/{id}/items/{item}/content`（转发 `/items/{id}/download`）、
   `GET /_wonton/api/media/sessions/{id}/stream`（转发 plan 的流地址）。
4. AirPlay/Chromecast 这类“接收端是另一台设备”的场景，核心另签发一个绑 LAN 地址的一次性绝对 URL（带短期令牌、只对被投的那一项有效）；回连不成立就退回局域网直连，外网未验证。

回环（`/streams`）属于客户端核的导出，不在服务端 API 里；服务端只发“给我一个能投出去的地址”的工具/接口。
- 适配器是**独立可替换的组件**，可以是一个只跑翻译的边车容器，也可以是插件主体里的一段；对核心是同一份 HTTP 契约。换掉 Immich 的适配器，核心一行不改。

理由：ADR-0015 要求核心不为具体插件开特例。三家都不是 Wonton 定的接口，Immich 和 Jellyfin 也没有开放标准（OpenSubsonic 算一个，但它只盖住音乐）。核心内置三家客户端意味着第三方插件要等核心发版；能力加适配器则新源自己带适配器进来。

对“每个组件能单独替换”的影响：换插件只换适配器，核心的长任务、队列、ViewModel、壳全不动；换壳不动协议；换核心只要重实现使用方那一侧，契约测试保它行为一致。代价是多一层进程与一次翻译，以及适配器作者要读契约文档。

没选的做法：

- **核心内置 Subsonic/Jellyfin/Immich 客户端**：发一次核心版才能接一种新源，第三方插件永远接不进来；三家接口的变更要核心跟着发版。
- **插件自己实现长任务**：每个插件各写一遍重试、断点、配额，行为分叉，Agent 要按插件分别接。
- **只用开放标准**：音乐能走 OpenSubsonic，相册和影视没有可用的开放标准，还是得为它们开特例。
- **行业做法照搬**（Plex 只接自家、Synology 只接自家、Infuse 把协议编进单体客户端）：三家都没有插件生态，能接多少种源由发版决定；Wonton 已有插件体系，照搬会丢掉可替换性。

### 3. 长任务的统一模型：上传与下载共用一套骨架，播放会话借用状态与失败分类

两类长任务：`Upload`（相册备份、收件）、`Download`（离线）；两者共用下面这套骨架，字段和状态机只有一份（`wonton-protocol` 的 `media` 模块），只在标“差异”的地方分叉。

**身份与任务号。** 每个任务带 `(person, device, task_id)`。`person` 是本机家人（ADR-0045/0088 的账号），`device` 是设备标识。设备走隧道时核心已按隧道 peer 认出设备（不需要另发 `_meta`）；HTTP 请求里显式带 `X-Wonton-Device`，供同一个人两台设备时区分。任务号由服务器核发，客户端核只引用。凭据一律按人取（第 6 节），不按设备。

**状态机。** 一份，上传与下载共用，播放会话也用它：

```
Queued → Preparing → Running ⇄ Paused{reason} / WaitingNetwork / WaitingPower
   ↘ Paused / Waiting…  ↘ Done | Partial | Failed{failure} | Cancelled
```

- `Paused{reason}` 是用户或 Agent 主动暂停，带原因（`user`、`agent`、`low_battery`…）。
- `WaitingNetwork`/`WaitingPower` 是系统条件不满足，条件恢复自动回队（`Queued`），到点重试走同一条，不算用户暂停。
- `Partial` 只给上传，且上传的目标是一**批**：一个备份任务带 `items: Vec<UploadItem>`（相对路径、大小、校验和、拍摄时间），逐项有 `Queued/Running/Done/Failed{failure}/Skipped`；失败项进任务的 `partial_failures`，全成 `Done`、全败 `Failed`、部分成 `Partial`。相册备份就是“一次备份 = 一个任务 + N 个 item”，不拆成几百个任务。
- 终态不可逆；再跑就是新任务（`retry_of` 指向父任务）。

**进度。** 统一 `{done, total, unit}`，`unit` 是 `bytes` 或 `items`；播放会话额外带 `position`、`duration`。客户端核限制上报频率：传输每 1 秒或每 256 KiB 一次，播放每 5 秒和 pause/seek/stop 时一次；服务器核再节流后给视图。`media.task.progress` 只推给此刻订阅着的 ViewModel（返回给壳），Agent 不订进度、要时用 `wonton__media_status` 轮询。

**断点与恢复。**

| 种类 | 断点 | 恢复 |
| --- | --- | --- |
| 上传 | 提供方侧的 `HEAD /uploads/{id}` 返回已收偏移；分片大小随提供方声明 | 从偏移续写；偏移对不上就重开一个上传 |
| 下载 | `.part` 文件加 `Range`/`If-Range`，带 ETag | 服务器支持 `Range` 就续，不支持（206 缺失或 ETag 变了）就从 0 |
| 播放 | 服务器核记的 `position` | 新会话从 `position` 起播；提供方不支持 seek 则从头 |

**重试与退避。** 只对“可能自己好”的失败重试：1/2/4/8/16/32/60 秒，最多 7 次，之后转 `Failed`。权限错、格式不支持、空间不足不退避，直接失败。每个任务独立计时，不在全局排一个队。

**失败分类。** 以 `wonton-protocol/src/file/mod.rs:97-142` 的 `FileFailure` 为底，**保留它的本地、传输、协议、上游四组**（`local_source_unavailable`、`local_permission_denied`、`local_storage_full`、`local_io`、`transport_unavailable`、`protocol`、`plugin_unavailable`、`upstream_error`），再加 `AuthenticationRequired`、`UnsupportedByDevice`、`RateLimited`、`MediaGone`、`ServerStorageFull`。服务端空间不足和手机空间不足分成两个码，壳才知道该去清哪边；`device_no_response` 只出现在工具结果里，不进任务失败枚举。失败即停，不藏错误：状态转 `Failed{failure}`，失败原文和类别都进视图与事件（`media.task.failed`），Agent 能看到完整原因。

**配额与清理。** 服务器核记两个账本：每个提供方的上传总量上限（按人）和每台设备的离线下载占用上限（按人按设备）。超限在 `Preparing` 就拒；拒的回执和 `QuotaView` 都分两项（`provider_upload`、`device_offline`），各带自己的 `used`/`limit`，壳据此出“去清服务器”还是“去清手机”。离线文件的清理只由用户或 Agent 显式发起；App 在系统存储告急时只提示，不自行删。

**后台限制。** iOS 的传输走 `URLSession` 后台会话：它不需要声明后台模式，系统在自己事件里把 App 拉起来并交回结果；只有批量枚举相册、整理这类不跑完会被打断的活才用 `BGProcessingTask`，那才要加 `processing`（现在只有 `audio`，`apps/ios/project.yml:69`）。`PHPhotoLibrary` 给的是内存数据，后台会话只能用文件，客户端核先落临时分片文件，传完删；空间不足先算再落。Android 传输走 `WorkManager` 加 `dataSync` 前台服务，一天总时长 6 小时（[FGS 超时](https://developer.android.com/develop/background-work/services/fgs/timeout)）；超过 6 小时的任务（大库首次备份）必须分批，批与批之间落回 `WaitingPower` 或 `Queued`。

**多设备。** 一人可以有多台设备，每台各跑各的传输任务，互不影响；**播放会话一人同时只有一个**，新设备开就是接管（第 4 节）。下载任务的配额按设备算，同一部片在两台设备上各占一份。

**两者的不同点：**

| | 上传 | 下载 | 播放会话 |
| --- | --- | --- | --- |
| 目标 | 提供方的 `upload` | 手机本地 `.part` 后改名 | 提供方的 `playback` |
| 断点粒度 | 分片 | 字节 | 位置 |
| 断点存在哪 | 提供方（`HEAD`） | 本地文件 | 服务器核 |
| 自动开始条件 | 充电 + Wi-Fi（可改） | 用户或 Agent 发起 | 用户或 Agent 发起 |
| 完成事件 | `media.task.done` | 同左，带本地路径 | `media.playback.finished` |
| 清理 | 无（已在提供方） | 手动/Agent | 无 |

两者共用 `MediaTask` 的类型、状态机、进度、退避、失败分类、配额和后台限制接口，只在上表几行分叉；播放会话只借状态与失败分类，不用 `MediaTask`；实现卡按“一张传输骨架卡 + 两张差异卡（上传、下载）+ 一张会话卡”拆，不按三块各拆一套。

没选的做法：

- **上传、下载各写一套**：两种重试和失败分类会分叉，Agent 要分三套接，界面上三处进度各长一个样。
- **只做传输、下载不归入**：下载和上传共用 `Range`/`.part` 与同一套退避，拆开就是抄一遍。
- **全局单队列**：一个卡的退避会堵住所有人；播放也要和备份抢队。
- **失败自动无限重试**：错误被藏住，用户看不出是密码过期还是格式不支持。

### 4. 播放会话：会话与队列在服务器核，壳只放

播放会话借用第 3 节的骨架（同一份人员/设备、状态、失败分类），但**不落任务表**：它有自己的表、自己的 `PlaybackSession` 与 `PlaybackViewModel`，`TaskKind` 不带 `Playback`。这样“任务列表”里不会同时出现一条任务和一条会话。

**事实源在服务器核。** 队列、当前位置、当前会话的设备归属存在 `crates/wonton-core`，壳不持有可写的副本，只渲染核心下发的 `PlaybackViewModel` 并出声、出画。理由：换设备要接着放、Agent 要问“在听什么”并遥控、重装不能丢（第 1 节结论 1）。驱动播放器的字节与解码留在客户端核与壳；核心只把字节原样中转，不缓冲、不落盘。

- 起播请求带 `person`、`item`、`device`、`DeviceProfile`、从上次 `position_ms` 起的起点，以及初始音轨/字幕选择 `audio`/`subtitle`（都可省，省了就由提供方按设备档挑）；核心据此向提供方要 `PlaybackPlan`，把 plan 与 `session.id` 一起回给客户端核。plan 的 `session` 由**提供方**发（上游会话号），和核心的 `PlaybackSession.id` 是两套，映射只存在核心。
- `plan.stream` 是给客户端的核心路由（由核心在返回前填好），提供方原址放在 `plan.origin_stream`，只给 `cast-url` 用，客户端不读它；
- 计划里带音轨和字幕轨两份 `MediaTrack` 列表，`ControlOp` 有 `SetAudio`/`SetSubtitle`（字幕可 `None`）：这两个由核心**重新向提供方要 plan** 再下发，新 plan 到位前客户端继续用旧轨，失败保留旧轨并报错；可 seek 范围以 plan 的 `seekable` 与 `duration` 为准。
- 队列改动（入队、插队、移除、跳转、清空、模式切换）都带本次读到的 `revision`，并在成功时把整条队列用 `PUT /queue` 推给提供方（`savePlayQueue` 的翻法），需要时带 `position` 与 `mode`；核心比对不相等就回 `409 stale_revision` 连同当前队列快照，客户端重新拉一次再重放操作。选 409 不选“服务端自动合并”：Agent 与两台手机同时改队列时自动合并会出谁也预料不到的次序，宁可让调用方重试。
- **一人同时只有一个活动会话。** 另一台设备发 `play` 时走接管：旧设备收到 `media.session.taken_over` 事件，壳停声、上报最后位置、转入只读。选“一人多活动会话”就得回答“锁屏上显示哪一个、scrobble 记给谁”，第一版不值得。
- 会话的终态带 `stop_reason`（`user`/`agent`/`taken_over`/`finished`/`error`/`removed`，最后一个是退出家庭时核心替他停的），停了不再改；重复的停止只把位置往后推。库里按人建一条唯一索引，只允许一条停因为空的活动会话。
- 位置上报：播放中每 5 秒一次，另外在 pause/seek/stop 各报一次；核心按会话节流后再转给提供方（Subsonic `scrobble`、Jellyfin `Sessions/Playing/Progress`）。节流放在核心不放在壳，免得每个壳各写一份、重试风暴直接打到插件。位置就是本会话的断点恢复点，恢复时以最近一次上报为准，不重头。
- 锁屏、通知栏、耳机按键由壳接系统：Android **只保留一个** `MediaSessionService`，现有的 `PageMediaService` 被新的播放服务吸收、页面媒体降为它的一个 renderer；iOS 走 `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter`，壳把按键翻成第 3 节的 `MediaAction` 回给核心。控制逻辑不在壳里重复实现。
- 画中画（PiP）两端壳各做一份，核心只保证进 PiP、出 PiP 不会停会话。
- 投屏分两轮：AirPlay 第一轮（系统播放器自带，壳把播放器指向核心签发的流地址）；Chromecast 第二轮，**接收端要能访问核心签的流地址，局域网可用，外网要公网入口，未验证**。投屏时扬声器在接收端，壳只报位置，不解除本地会话。
- 车机不在第一轮：CarPlay 要单独授权项，Android Auto 要 `MediaLibraryService`，工作量各自是一张 L。先做到“配对、锁屏、耳机可用”，车机列在最后一批。
- **取代 ADR-0014 的“Subsonic 原生音乐页”一节**（那里写着“不做原生音乐页”）；ADR-0005 的“状态都在客户端核”由本篇按第 1 节补一条：跨设备的离线长任务与播放会话事实源在服务器核，其余仍按 0005。
- 与 #585 的关系是收编，不是并存，也不是删：网页 `<audio>` 的媒体会话降为一种 renderer。`PageMediaOwner.kt:42-53` 的单弱引用改成由客户端核统一收口，网页只上报“这一页在放什么”。第三方插件页继续走这条路；音乐原生页落地后，网页 renderer 只给还用 `<audio>` 的页留着。

没选的做法：
- 会话状态放客户端核：换设备接不上、Agent 看不到、重装即丢（第 1 节已否）。
- 让插件网页自己管队列：就是 #585 的现状，页面一死会话就死。
- 每次位置上报都直打插件：频率高、失败无退避，私有网络下会把插件打满。

### 5. Agent 原生：工具、确认、命令通道

Agent（Hermes，家里服务器上）与壳走同一条核心 API，不给 Agent 开旁路，权限与确认也走同一套。

**MCP 工具**（核心提供，供 Bot 调用）：

| 工具 | 做什么 | 发不发确认卡 |
| --- | --- | --- |
| `wonton__media_status` | `{person?}`：查某个人或自己的任务与播放会话、进度、失败原因 | 不发（只读，ADR-0024 第 2 步的“不敏感”） |
| `wonton__backup_now` | 让指定设备立即跑一次备份 | 发，发给本人 |
| `wonton__media_download` | 给某设备排一个离线下载 | 发，发给本人 |
| `wonton__media_play` | 在某人正在用的设备上开播或接管 | 不发（可撤，坏了再点一次） |
| `wonton__media_control` | pause/resume/seek/next/prev/音量 | 不发 |
| `wonton__media_queue` | 读队、入队、跳转 | 不发（可撤） |
| `wonton__media_sessions` | `{person?}`：列出这个人所有设备的会话与能力 | 不发 |
| `wonton__media_task` | `{task, op: pause\|resume\|cancel\|retry}` | 不发（可撤） |
| `wonton__media_delete` | 删某台的离线文件 | 发（删除类） |

- 删离线文件、停别人（非说话人本人）的会话，按 ADR-0024 的删除类走，发卡片。
- `person` 只能填本轮 `app.wonton/speakers` 里的人（ADR-0114 第 2 项）；共享 Bot 里省略就取唯一的说话人，多人时必填。
- 每张卡片、结果卡的模板与深链全部引用 ADR-0114 的第 5 项（卡片模板）与第 6 项（深链），本篇不重写；身份取 `speakers` 见第 2 项。
- 工具作用于“这个 Bot 的数据主体”；共享 Bot 按上一条取说话人；私人 Bot 忽略 `person`（只能是本人）。日志与卡片都记“替谁”。

**到设备的命令通道。** 核心不直连手机，设备拉：

1. 设备前台或刚有活动时跑 `GET /_wonton/api/media/commands?after=<seq>` 长轮询，最长挂 25 秒；有命令就回，带 `seq`。
2. 睡着的设备按 ADR-0098 第 2 节的写法用推送唤醒（iOS 静默推送，Android 隧道或高优 FCM），推里只有 `seq`，不带内容。
3. 设备收到后重新拉命令；30 秒没回应，工具结果报 `device_no_response`，原因照 ADR-0098 第 6 节列全（关机、没网、被划掉、被系统限流），不挑一个。

命令是幂等的（同一个 `command_id` 重放无副作用）；设备按 `seq` 单调回执，核心顺手清掉旧命令。

事件分两路：

- **进度只推在线订阅**（长轮询/推送的当前订阅者），最新值覆盖旧值，不进队列——1 秒一次的进队会把终态冲掉。
- `media.task.done`/`media.task.failed`/`media.playback.finished`/`media.session.taken_over` 走 ADR-0114 第 3 项的事件队列（带 `dedupe_key`），按人投递、能唤醒 Agent。

事件的“发送方”是核心本身：0114 的队列按发件插件限流（每插件 10 条/分），核心事件不受插件限流；`DeliveredEvent.plugin_id` 记提供方插件（有就记，队列的收件人是这个人而不是插件）。按人通知见 0114 第 4 项，说话人见第 2 项，深链见第 6 项。

没选的做法：
- Agent 直发推送、自己记设备状态：多一套状态，和壳看到的不一致。
- 设备维持一条长连接：耗电，且 iOS 保不住（ADR-0032 的连接恢复已说明）。
- 用 MCP 资源代替工具：各家 Agent 订阅支持参差，且播放控制本来就要调（同 ADR-0098 第 4 节）。

### 6. 家人隔离与凭据

上游凭据按人各一把，存在提供方侧，核心只持“人 ↔ 任务/会话”和给提供方的那把能力凭据（能力 `media_source` 的 `Credentials::Required`，与 `personal_data` 同形，`plugin/capabilities.rs:120-128`）。

| 块 | 上游身份 | 凭据放哪 |
| --- | --- | --- |
| 相册 | 每人独立 Immich 库，每人一把 ingest key | 提供方（Immich 插件）侧，按 `X-Wonton-User` 取 |
| 音乐 | Navidrome 每人一个用户 | 同上 |
| 影视 | Jellyfin 每人一个用户，用 Quick Connect 登录（待实测，跑不通就首次输一次密码） | 同上 |

- 核心每次转发带 `X-Wonton-User`，提供方按人取自己那把。核心不存上游口令、不把别人的凭据发给别人。
- 换手机不搬凭据：凭据在服务器侧和提供方侧，新手机重新配对即接上，旧的设备条目失效、不删凭据。
- 退出家庭：核心先取消其全部任务、结束其全部会话，再 `revoke {name}` 让提供方注销上游凭据；下载副本留不留由本人选，账号删了就清。`revoke` 的具体形状引用 ADR-0114 第 1 项，本篇只要求“退家庭必须同时取消任务与会话”这一条。
- “家庭”账号只当共享相册的库主，和某个具体人的任务、会话无关；它不持有手机上的备份会话。

没选的做法：核心存全部凭据（与 ADR-0114 第 1 项同，核心会变成凭据库，且与“凭据按人、提供方侧”相反）；一人一份插件实例（ADR-0045 已否）。

### 7. 范围

做：

- 一套长任务模型（上传与下载共用第 3 节的身份、状态、进度、断点、退避、失败分类、配额；播放会话借用同一套状态与失败分类）；
- 能力 `media_source` 与契约测试（两个假提供方），核心当使用方，三个插件各出一个薄适配器翻上游接口（Immich `POST /assets` 与 `bulk-upload-check`、Subsonic `stream`/`scrobble`/`savePlayQueue`、Jellyfin `PlaybackInfo`/`Sessions/…`）；
- 服务器核的任务与会话、命令通道、MCP 工具；客户端核的执行与 ViewModel；两壳的原生播放、上传、下载、锁屏与 PiP；
- 音乐、影视各自的原生播放界面，相册的原生浏览界面（结论已由主人定，本篇只给底座；相册的浏览契约见第 9 节）。

不做：

- 不做三份重复的设计：相册、音乐、影视只各自写适配与界面；
- 不自研媒体服务端，不改上游源码（Jellyfin/Immich/Navidrome 只看行为与文档）；
- 不做直播与 DLNA/UPnP；
- 不做声学推荐（推荐归各块的 Agent 工具）；
- 第一轮不做 Chromecast（第二轮）与车机（最后一批）；AirPlay 第一轮做但只做壳自带能力，不扩展。

### 8. 兼容与迁移

- 全程只加不改，但**新字段会让旧核心拒装**：`Provides` 本身是 `#[serde(flatten)]` 的 map 容器（没有 `deny_unknown_fields`），`media_source`
  会落进 `capabilities` map、再被 `ProviderService` 的 `deny_unknown_fields` 因为 `operations`/`kinds` 拒掉。所以新清单要升 `spec` 并在
  `spec_floor.rs` 登记，旧核心按版本号拒装并提示升级，不是静默忽略；别去给 `Provides` 加 `deny_unknown_fields`。
- 新 App + 旧核心：新 App 看不到 `Action::Media` 的订阅，音乐、影视原生页退到插件网页；只有壳的改动，不崩。
- 旧 App + 新核心：新协议字段不认识，任务页照旧，原生页在旧 App 里不出现（ADR-0095 的“不认识的按钮不显示”）。
- #585 的网页媒体会话在收编期间保持可用，直到音乐原生页与第三方页的 renderer 都验过（见“待验证”）。
- `FileFailure` 的码只增不改；服务端空间不足改用新码，旧壳落到通用文案。

### 9. 相册：原生浏览、去重和实况照片（2026-11-27 补充）

主人 11-27 定相册也做原生界面：时间线、相册、共享相册、家庭相册、大图、视频和实况照片播放、搜索都在两端原生画，
手感对标 Google Photos 和 Synology Photos；Immich 网页降为管理和高级入口。手机自动备份同样自己做（第 3 节的上传任务）。

**浏览走 `media_source` 的新操作 `library`，客户端核不直讲 Immich。** 第 2 节的读路径例外只给 OpenSubsonic 这个开放标准；
Immich 没有标准，客户端核直讲它就是给一个具体插件开特例（ADR-0015），换一个相册插件就要改客户端核和发 App。
`GET /items` 的通用列表也不够：没有按月分桶、尺寸、实况、归属和相册，画不出网格。`library` 的面按相册应用共有的几样定：

| 提供方路径 | 方法 | 回包（`wonton-protocol::media::library`） |
| --- | --- | --- |
| `/library` | `GET` | `LibraryFeatures`：接哪些写操作（`actions`）；只读的源回空表，壳不出对应按钮 |
| `/library/timeline` | `GET` | `Timeline`：按拍摄月分桶（`YYYY-MM`、张数），新的在前 |
| `/library/timeline/{month}?cursor=` | `GET` | `AssetPage`：这个月的条目，新的在前 |
| `/library/albums` | `GET` | `Albums`：这个人能看到的相册，带归属（`me`/`household`/`person:<用户名>`）和是否共享 |
| `/library/albums/{id}?cursor=` | `GET` | `AlbumPage` |
| `/library/search?q=&people=&place=&from=&to=&kind=&favorite=&cursor=` | `GET` | `AssetPage`；`q` 按画面内容搜 |
| `/library/people` | `GET` | `People`：有名字的人物，搜索页按人筛 |
| `/library/people/{id}/thumbnail` | `GET` | 人物头像的字节 |
| `/library/items/{id}` | `GET` | `PhotoDetail`：EXIF、地点、人物 |
| `/library/items/{id}/content?rendition=` | `GET` | 字节；档位 `thumbnail`/`preview`/`original`/`playback`/`live`，`Range` 原样过；条件请求命中回 304、越界回 416，照常转，不是失败 |
| `/library/actions` | `POST` | 体 `LibraryAction`（收藏、建相册、加进/移出相册、共享给家人、进回收站、恢复），回 `LibraryActionResult` |

- 服务器核对 `library` 只转发：按设备归属的人填 `X-Wonton-User`、换上提供方签的能力凭据、`Range` 原样过，不缓存、不落盘、
  不改写回包。客户端走 `/_wonton/api/media/library/…`，后半段就是上表的提供方路径。用哪一家照“选哪个提供方”的统一规则
  （`notes/0113-interfaces.md`）：带 `?source=<插件 id>` 用它，没带用主人定的默认，只有一家就用它，多家又没定默认回
  `source_required`，不替人挑；这个人用不了的那家不算在“多家”里。
- 这个人没被授权打开这个插件（ADR-0088 的应用清单）回 `permission_denied`，和打开插件网页是同一个判断。
- 客户端核持有浏览的状态：游标、按日分组、选择集、缩略图磁盘缓存（按条目 id 和档位，有上限、按最近使用淘汰）；壳只画网格、
  手势和转场（ADR-0120 手写白名单里的“相册看图”）。字节经客户端核的本机回环交给原生图片加载器和播放器，不经绑定层搬。
- 视频和实况照片的动态部分在大图页直接播（`playback`、`live` 档），不开第 4 节的播放会话：相册里的短视频不进队列、
  不跨设备接着放。
- 原生页上本人点的写操作可撤、只动自己能动的，不发确认卡片（ADR-0024）；Agent 改相册走插件自己的 MCP 工具，确认照第 5 节。
- 失败回 `MediaFailure` 信封，状态码和码对应如下；上游把“看不到”“不存在”也说成 400 的，提供方按意思归类，不归成 `protocol`：

  | 情形 | 状态码 | 码 |
  | --- | --- | --- |
  | 请求本身不合法：缺参数、编号格式不对、认不出或不在 `LibraryFeatures` 里的动作 | 400 | `protocol` |
  | 凭据不对 | 401 | `authentication_required` |
  | 这个人看不到或改不了这一条、这个相册 | 403 | `permission_denied` |
  | 条目、相册不在了 | 404 | `media_gone` |
  | 上游限流 | 429 | `rate_limited` |
  | 上游出错 | 502 | `upstream_error` |
  | 上游连不上、没应答 | 503 | `plugin_unavailable` |

  写操作逐条判断：做不了的那几条（别人的照片、不在了的）进 `LibraryActionResult.failed`、各带码，其余照做；上游整批全有或
  全无时提供方先挑出做不了的再做。整批都做不了才回失败信封。
  提供方不认识的动作回 `protocol`，不静默忽略；部分成功逐条回 `failed`。
- 每人独立库：提供方按 `X-Wonton-User` 用这个人自己的上游账号，回包只含他能看到的（自己的、家人共享给他的、共享相册）。
  别人的条目带 `owner`（本机用户名），壳据此标出来源。“家庭”账号当家庭相册的库主（第 6 节），归属写 `household`，壳单列。
- 上游账号首次用到时由提供方建，和网页 OIDC 自动开户同一个时机（核心只把授权过的人转过来）；ADR-0114 第 1 项的
  `member_accounts` 落地后，核心在授权时先调 `add`，首次建号这一步就提前，提供方的建号保持幂等。

**去重。**

- 手机上的客户端核给每一项算 SHA-1，写成 `sha1:<小写 hex>` 放进 `UploadItem.checksum`；按（系统资源 id、修改时间）缓存，
  不每次重算。和 Immich 的去重同一种算法，提供方不用再算一遍。
- 建备份任务前先问提供方 `POST /uploads/check`（一次最多 1000 个），已在这个人库里的不进任务：换手机、重装、两台手机里的
  同一张都不重传。回收站里的算不算“已在库里”归提供方（Immich 的 `bulk-upload-check` 语义）。
- 收尾时仍可能撞上重复（检查和上传之间别处传了同一张），`finish` 回 `duplicate: true`，核心把这一项记成 `Skipped`，不算失败。
- 去重只决定传不传，不删手机上的东西；“备份后释放手机空间”是壳列出已备份项、用户确认后交给系统删。
- 每一项都带拍摄时间（`taken_at`，手机相册的创建时间）。相册类提供方按它排时间线，没带就回 `protocol` 拒收，不拿上传时刻
  顶上：顶上的话旧照片会全堆在上传那天。

**实况照片。** iOS 的 Live Photo、Android 的动态照片是一张图加一段视频，各是一个 `UploadItem`。动态那段排在前面；
静态那项带 `live_video_checksum`，提供方按这个校验和找到已收的那段再关联（Immich 的 `livePhotoVideoId`）。
动态那段没传成时静态那项不传，跟着标同一个失败；重试时两段一起再来，不会留下一张丢了动态部分的图。

没选的做法：

- **客户端核直讲 Immich 接口**：见上，给具体插件开特例；还要把每人的 Immich key 发到手机上。
- **只用 `GET /items` 的通用列表**：字段不够画网格，按月分桶、实况、相册、共享都得另加。
- **原生页内嵌 Immich 网页**：主人已定原生；网页的滚动位置、手势、离线缓存都不在我们手里，进程被杀就回到入口。
- **核心缓存缩略图**：核心只中转不缓冲（第 1 节）；缓存放客户端核，命中时连网都不用。

## 媒体源契约

提供方按能力 `media_source` 实现路径；使用方是核心，核心不认具体插件。凭据由提供方签给核心，每个请求带 `X-Wonton-User`（任务或会话的主人）。

| 路径 | 动作 | 语义 |
| --- | --- | --- |
| `GET /_wonton/capabilities` | 探测 | 返回 `operations`（`upload`/`download`/`playback`/`list`/`library`）与 `kinds`（`photo`/`video`/`audio`）；核心据它决定哪些操作可用 |
| `GET /items?kind=&q=&album=&artist=&series=&from=&to=&cursor=` | 列条目 | 分页列这个人能看到的条目，给 Agent 找内容；按 `title` 排，`cursor` 在同一次查询的参数下稳定；回 `items` 与下一页 `cursor` |
| `POST /uploads` | 开始上传 | 体是 `UploadStart`（相对路径、大小、校验和、拍摄时间）；返回 `upload_id` 与已收字节数（为 0 或断点位置） |
| `HEAD /uploads/{id}` | 查断点 | 返回 `Upload-Offset`；不存在时 404，核心从头开始 |
| `PATCH /uploads/{id}` | 续传 | 体是从 `Upload-Offset` 起的字节，`Content-Range` 校验；成功回 200、冲突回 409，都带当前偏移（`UploadOffset`） |
| `POST /uploads/{id}/finish` | 收尾 | 提供方校验后入库，返回 `item_id`；收尾时才发现重复回 `duplicate: true`（第 9 节） |
| `POST /uploads/check` | 去重检查 | 体带校验和（一次最多 1000 个），回已在这个人库里的那些和它们的 `item_id`（第 9 节） |
| `DELETE /uploads/{id}` | 放弃 | 幂等 |
| `POST /playback` | 取播放计划 | 体带 `item`、`DeviceProfile`、`position_ms`、`audio?`、`subtitle?`（同 `PlaybackRequest`；切轨用同一 `item`、当前 `position_ms` 与新轨号重取）；返回 `PlaybackPlan`（直连 URL 或转码流、音轨字幕、可 seek 范围、过期时间） |
| `POST /playback/{session}/progress` | 报进度 | 体带位置毫秒与墙钟；提供方按上游约定写（scrobble、`Sessions/Playing/Progress`） |
| `PUT /playback/{session}/queue` | 存队列 | 体带 `entries`、`position`、`mode`；幂等，成功 204；提供方不支持的队列字段忽略 |
| `POST /playback/{session}/stopped` | 结束 | 带最终位置与原因 |
| `GET /items/{id}/segments` | 章节/片头片尾 | 回 `segments`（类型、起点、终点）；没有就 404。plan 里带了就以 plan 为准，起播后新抓到的以新抓的为准 |
| `GET /items/{id}/download` | 下离线 | 支持 `Range`/`If-Range`，回 206 与 `Content-Range` |
| `GET /items/{id}/artwork?type=&width=` | 封面、海报、缩略图 | `type` 取 `primary`（不带时）、`backdrop`、`thumb`、`logo`，认不得的按 `primary`；`width` 是要的像素宽，提供方按它缩、不放大。回图片字节，带 `Content-Type`；没有就 404。原生页的浏览靠它和 `list`，手机连不到 `artwork_url` 里提供方的内部地址 |
| `/library/…` | 相册浏览 | 声明了 `library` 操作的提供方实现，路径和回包见第 9 节的表 |
| `POST /revoke` | 收回 | 体只带 `name`；提供方只注销上游凭据、停掉上游会话（任务表在核心，核心自己取消任务、结束会话） |

失败回 `MediaFailure` 信封（`code`、`message`、`retry_after?`），码即下面失败类别的名字。

提供方回包里的地址（`PlaybackPlan` 的 `stream`、`origin_stream`，`MediaItemRef.artwork_url`）写相对这个提供方服务根的路径，可带查询串：提供方不知道核心怎么叫它，手机也进不了插件网段。写成绝对地址时核心也只取路径和查询串，主机名不认。

契约测试照 `plugin/tests/personal_data_contract/mod.rs:1-10` 的样板：两个假提供方（一个只实现 `upload`、像相册；一个只实现 `playback`/`download`/`list`、像点播）跑同一组用例，断言核心的 ViewModel、退避次数、断点续传、换提供方时客户端壳不改。

## 失败类别

码分五组：本地、传输、协议、上游、平台。前三组来自 `FileFailure` 原样保留，后两组是新加的；`device_no_response` 不是任务的失败，只在工具结果的失败体里单独带：
`{code: "device_no_response", message, reasons: [DeviceNoResponse]}`，`reasons` 的取值照 ADR-0098 第 6 节列全（关机、没网、被划掉、被系统限流）。

| 类别 | 何时出现 | 壳怎么显示 | 退避 |
| --- | --- | --- | --- |
| `local_source_unavailable` | 手机上的源被删/改了 | “源文件不见了”，跳过这一项 | 不退避 |
| `local_permission_denied` | 系统权限没给 | “去授权”，带跳转设置 | 不退避 |
| `local_storage_full` | 手机空间不足 | “手机空间不足”，列占用与清理 | 不退避 |
| `local_io` | 本地读写失败 | “本机出错”，带重试 | 1 次 |
| `transport_unavailable` | 没有网、隧道断 | 进 `WaitingNetwork`，恢复后从断点续 | 1/2/4/8/16/32/60 秒，最多七次 |
| `protocol` | 上游回了看不懂的东西 | “对不上”，带重试 | 同上 |
| `authentication_required` | 上游凭据失效或被收回 | “需要重新登录”，带跳转入口 | 不退避 |
| `permission_denied` | 上游拒了这个人的权限 | “没有权限” | 不退避 |
| `unsupported_by_device` | 格式或 `DeviceProfile` 不被接受 | “这台设备放不了”，给转码按钮 | 不退避 |
| `rate_limited` | 提供方回 429 | “限速中，N 秒后重试” | 按 `retry_after`，不计入七次 |
| `media_gone` | 文件被删或移走 | “已不在库里”，该项标 `Failed` | 不退避 |
| `plugin_unavailable` | 插件没装/没起来 | “插件没在跑”，带重试 | 1/2/4 秒，最多三次 |
| `upstream_error` | 上游 5xx | “服务器出错”，带重试 | 同传输 |
| `server_storage_full` | 服务器空间不足 | “家里硬盘满了”，列占用 | 不退避 |
| `source_required` | 能做这件事的媒体源不止一家，主人没定默认 | 请用户选一家，带 `source` 再来 | 不退避 |
| `stale_revision` | 队列被别处先改了，这次按位置的改动对不上 | 换上当前队列，请用户看过再改 | 不退避 |
| `suspended_by_system` | 平台暂停（iOS 挂起、Android 配额到） | 进 `Paused{reason}`，不报错；它只在 `Paused` 里用，不进 `Failed` | 恢复时从断点续 |

退避用完标 `Failed`，不藏也不无限重试；手动重试重置计数。

## 代价与风险

- 每个插件都要出一个薄适配容器（Immich、Subsonic、Jellyfin 三张仓内卡），多一层进程与部署面；状态和命令多经一次翻译（延迟未量化）。
- 播放会话全在服务器核：手机没网时切歌、拖进度要等隧道或局域回包，感受取决于 ADR-0032 的恢复速度；核心宕机则会话断，客户端需重开会话（未验证体感）。
- Chromecast 接收端要能回连核心地址：局域网可用，外网要公网入口或中继，未验证。
- iOS 后台传输只靠 `URLSession` 后台会话（不需要后台模式）；批量枚举相册要 `BGProcessingTask` + `processing`，能不能批未核实。
- Android `dataSync` 一天六小时封顶，大库首次备份必然跨天分批；播放要用 `MediaSessionService`，前台服务类型选错会被系统杀。
- “每个插件一个适配容器”意味着每接一个新源就多一份要维护的翻译层；上游改接口时适配器会坏，坏在插件仓里、按插件发版修，核心不跟着发；契约测试保核心一侧不变。
- “一个家人同时只有一个活动会话”：同一个人在两台设备上同时放（如给孩子的平板放、自己手机也放）会互相接管；第一版接受，按真实反馈再定。

## 没选的做法

| 做法 | 为什么没选 |
| --- | --- |
| 长任务全放服务器核（连执行也放） | 驱动不了手机上的播放器和系统能力，每个动作一次往返；字节也要多绕一跳 |
| 长任务全放客户端核 | 重装即失、换机丢队列，Agent 看不到，多设备队列不成立 |
| 核心内置三家客户端（不做 `media_source`） | 每加一个源都要核心发版，第三方插件无法自己接；且三家里只有 Subsonic 是开放标准 |
| 只用开放标准，没标准的让上游仿 Subsonic | 相册、影视的字段、鉴权、错误码和 Subsonic 对不上，硬套反而处处是特例；不如定自己的能力，让插件翻 |
| 播放仍用网页 `<audio>`（`music/plan.md` v1 的第 1 节） | 会话跟着页面活，锁屏、投屏、车机、持续播放都受制；主人已定改为播放会话与队列在核心、原生出声 |
| 原生播放器交给外部播放器 App | 锁屏、进度、离线、队列都不在我们手里，商业差距缩不了（`notes/video/plan.md` 问题 1） |
| 车机进第一轮 | CarPlay 要 Apple 授权、Android Auto 要 `MediaLibraryService`，周期长；放最后一批 |
| 每块各写一份 | 三块形状完全一样，接口会分叉 |

## 待验证

1. **Quick Connect 整链路**：新用户能不能从我们 App 里打开 Jellyfin 的 Quick Connect 页并免密登录；跑不通就退第一次密码（卡 `1k-media-jellyfin` 在测）。验法：真机走一遍，记录每步耗时和失败点。
2. **iOS 后台切上传**：`URLSession` 后台能否在 `fetch`/`processing` 模式下传完 1 GB（`notes/photo/plan.md` 卡 4），真机测。
3. **`dataSync` 配额**：真机连续传两小时测实际能否跑满（`apps/compose/androidApp/src/main/AndroidManifest.xml:96-101` 现无此类型），记录系统回调。
4. **Chromecast 回连**：局网与外网各一次，看核心地址是否可达。
5. **接管语义**：两台真机同一人同时放，测旧端收 `TakenOver` 后的锁屏与通知是否即清。
6. **播放器格式集**：iOS/Android 各真机把可直连格式逐个试，调的列表交给 `notes/video/plan.md` 卡 16 定。
7. **`#585` 降为 renderer 后第三方页**：抓现成第三方页，确认 `renderer` 不抢它的 `navigator.mediaSession`（`crates/wonton-client/src/pages/media/mod.rs:1-133`）。

## 接口影响

- `crates/wonton-protocol`：新 `media` 模块，含能力名、头、路径、`MediaViewModel`/`PlaybackViewModel`/`MediaAction`。字段级见 `notes/adr0113-interfaces.md`。
- `crates/wonton-protocol`：`media::library`（相册浏览的线上形状）、`media::uploads`（上传开始、收尾、去重检查的线上形状），`MediaOperation` 加 `library`（第 9 节）。
- `crates/wonton-core`：能力表加 `media_source`，核心侧提供方客户端与两个契约测试假提供方；`/_wonton/api/media/{tasks,sessions,queue,streams,commands}`；MCP 工具与事件。
- `crates/wonton-client`：`Action::Media`、`subscribe_media`/`subscribe_playback`、`player_state`、`_meta` 键；媒体页的 `#585` 会话改成 renderer。
- `wonton-docs`：`plugin-spec.md` 补 `media_source` 契约与错误码，清单 `provides.media_source` 字段。
- 壳：`apps/ios`、`apps/compose` 的原生声画与后台能力（`URLSession` 后台、`Media3 ExoPlayer`、`AVPlayer`）。
- 全部只加不改，没有破坏性的接口改动。#585 的 `PageMediaOwner` 单弱引用改成由客户端核收口是壳内实现，不动协议，归实现卡。

## 待定

1. **车机进不进第一轮**：进则需要 CarPlay 授权申请排期、Android Auto 的 `MediaLibraryService`，估 XL；不进则第二轮。推荐不进。
2. **Chromecast 接收端走哪条回连通道**：公网入口、中继、只许局网三选。推荐先只局网，外网与 ADR-0003 中继一起定。
3. **ADR-0114 冲突项**：本篇直接用 0114 的第 3 项（事件与唤醒）、第 6 项（深链）名字；若 0114 事件载荷字段与本篇 `MediaFailure` 撞，以 0114 为准、本篇改引用。底座线复审时定。
4. **离线下载配额默认值**（每人每设备 GB）：本篇未定，交给拆卡时按真机容量实测后填，暂定不限、只显占用。
