# ADR-0113 接口清单（前半：`wonton-protocol` 的 `media` 模块）

只改接口的 PR 的全部改动。**全部只加不改**，没有破坏性项；ADR 正文列了为什么不上破坏性改动（#585 的收口在壳内）。字段级照 ADR-0113 第 3、4 节，类型用 Rust 写。

所有要过壳的类型都照 `file/mod.rs:291-320` 的写法加 `#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]`（枚举加 `uniffi::Enum`）；下面为了可读没有逐条重复。

模块位置：`crates/wonton-protocol/src/media/`，`lib.rs` 加 `pub mod media;`，按职责分 `mod.rs`（常量与入口类型）、`task.rs`、`session.rs`、`view.rs`。

## `media/mod.rs`

```rust
/// 能力名，插件清单 `provides.capabilities.media_source` 与能力表共用这一串。
pub const MEDIA_CAPABILITY: &str = "media_source";
/// 复用 `personal_data` 已有的那一个（`personal_data.rs:22`），不另定义。
pub use crate::personal_data::WONTON_USER_HEADER;
/// 同一个人两台设备时区分；不打用户表。
pub const WONTON_DEVICE_HEADER: &str = "X-Wonton-Device";

pub const PATH_CAPABILITIES: &str = "/_wonton/capabilities";
pub const PATH_UPLOADS: &str = "/uploads";
pub const PATH_PLAYBACK: &str = "/playback";
pub const PATH_ITEMS: &str = "/items";

/// 一条媒体条目的不变量；原生页要的展示字段全在这里，提供方内部 id 只回传给提供方。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MediaItemRef {
    pub id: String,
    pub kind: MediaKind,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub artwork_url: Option<String>,
    pub duration_ms: Option<u64>,
    pub series: Option<String>,
    pub index: Option<u32>,
    pub played: bool,
    pub starred: bool,
}

/// 设备能力档：少数说死要比整套更好维护（编解码、HDR、最大码率）。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeviceProfile {
    pub direct_play: Vec<String>, // MIME
    pub max_bitrate: u32,
    pub hdr: bool,
    pub container: Vec<String>,
}

## `media/task.rs`

```rust
pub type TaskId = Uuid;
pub type Person = String;
pub type DeviceId = String;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind { Upload, Download }



#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Preparing,
    Running,
    Paused(PauseReason),
    WaitingNetwork,
    WaitingPower,
    Done,
    Partial,
    Failed(MediaFailure),
    Cancelled,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason { User, Agent, LowBattery, SuspendedBySystem }

/// 失败的类别就是协议面，不是文案；壳按类别出字。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MediaFailure {
    pub code: MediaFailureCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MediaFailureCode {
    LocalSourceUnavailable,
    LocalPermissionDenied,
    LocalStorageFull,
    LocalIo,
    TransportUnavailable,
    Protocol,
    AuthenticationRequired,
    PermissionDenied,
    UnsupportedByDevice,
    RateLimited,
    MediaGone,
    PluginUnavailable,
    UpstreamError,
    ServerStorageFull,
    SuspendedBySystem,
}

/// 一条长任务。三类共用；`target` 与 `progress.unit` 各自成立。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    pub kind: TaskKind,
    pub person: Person,
    pub device: DeviceId,
    pub target: TaskTarget,
    pub state: TaskState,
    pub progress: Progress,
    pub retry_count: u8,
    pub retry_of: Option<TaskId>,
    pub created_at: Timestamp,
    /// 只在上传里有；逐项失败原文
    pub partial_failures: Vec<FailedItem>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TaskTarget {
    Upload(Vec<UploadItem>), // 一批文件；相册备份 = 一个任务 + N 个 item
    Download(MediaItemRef),
}

/// 上传的一项：路径、大小、校验和、拍摄时间，都是提供方要的。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UploadItem {
    pub item_id: String,
    pub path: String,
    pub size: u64,
    pub checksum: Option<String>,
    pub taken_at: Option<Timestamp>,
    pub state: ItemState,
}

/// 逐项状态；`Partial` 由这里聚出来，不靠猜。
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ItemState { Queued, Running, Done, Skipped, Failed(MediaFailure) }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FailedItem { pub item_id: String, pub path: String, pub failure: MediaFailure }

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Progress { pub done: u64, pub total: Option<u64>, pub unit: ProgressUnit }

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ProgressUnit { Bytes, Items }

## `media/session.rs`

```rust
pub type PlaybackSessionId = Uuid;
pub type Revision = u64;

/// 队列自带 revision；所有改动带读到的 revision，不等就 409。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Queue { pub revision: Revision, pub entries: Vec<MediaItemRef>, pub cursor: Option<usize>, pub mode: QueueMode, pub sleep_at_ms: Option<u64> }

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode { Sequential, Shuffle, RepeatAll, RepeatOne }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackRequest {
    /// 从上次位置起播；0 表示从头
    pub position_ms: u64,
    pub item: MediaItemRef,
    pub device: DeviceId,
    pub person: Person,
    pub profile: DeviceProfile,
}

/// 提供方给的核心可播计划。`session` 是上游会话号，和核心的 `PlaybackSessionId` 是两套，映射只在核心。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackPlan {
    pub session: String,
    pub stream: ProviderStream, // 提供方原址，核心改写成自己的路由
    pub segments: Vec<Segment>,
    pub audio: Vec<MediaTrack>,
    pub subtitles: Vec<MediaTrack>,
    pub seekable: bool,
    pub duration_ms: u64,
    pub expires_at: Timestamp,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ProviderStream { Direct(String), Transcoded(String) }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MediaTrack { pub index: u16, pub language: String, pub label: Option<String>, pub kind: TrackKind, pub selected: bool }

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind { Audio, Subtitle }

/// 章节、片头片尾。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Segment { pub kind: SegmentKind, pub start_ms: u64, pub end_ms: u64 }

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind { Chapter, Intro, Outro }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackSession {
    pub id: PlaybackSessionId,
    pub person: Person,
    pub device: DeviceId,
    pub item: MediaItemRef,
    pub state: TaskState,
    pub position_ms: u64,
    /// 起播时带回的 plan；过期由核心重取
    pub plan: Option<PlaybackPlan>,
}

/// 建任务用的入参：`options` 是各轴自己的开关（只在传输、只在充电、转码率上限）。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskCreate { kind: TaskKind, person: Person, device: DeviceId, target: TaskTarget, options: TaskOptions }

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TaskOptions {
    pub wifi_only: bool,
    pub charging_only: bool,
    pub max_bitrate: Option<u32>,
    /// 相册范围（可选）；只传相册不传 items 时由设备枚举
    pub albums: Vec<String>,
    /// 同时在跑几项，1–4
    #[serde(default = "one")]
    pub concurrency: u8, // 1–4
    /// 立即备份：忽略 wifi/充电条件一次
    pub force_now: bool,
}

## `media/view.rs`

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ControlOp { Play, Pause, Resume, SeekMs(u64), Next, Prev, Volume(u8), SetAudio(u16), SetSubtitle(Option<u16>) }

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum StopReason { User, Agent, TakenOver, Finished, Error }

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum QueueOp { Read, Enqueue { at: Option<usize>, item: MediaItemRef }, Remove { index: usize }, Jump { index: usize }, Clear, SetMode(QueueMode) }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DoneItem { pub item: MediaItemRef, pub local_path: Option<String> }

/// 壳→核的动作与核→设备的命令是同一个类型。
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MediaAction {
    StartUpload { task: TaskId },
    StartDownload { task: TaskId },
    Playback { session: PlaybackSessionId },
    Control { session: PlaybackSessionId, ctl: ControlOp },
    Stop { session: PlaybackSessionId, reason: StopReason },
    Queue { person: Person, revision: Revision, op: QueueOp },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeviceCommand { pub command_id: Uuid, pub seq: u64, pub body: MediaAction }

/// 壳渲染任务用；文案只放类别不给句子（壳按类别出字）。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MediaViewModel { pub tasks: Vec<TaskView>, pub quota: QuotaView }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskView {
    pub id: TaskId,
    pub kind: TaskKind,
    pub title: String,
    pub state: TaskState,
    pub progress: Progress,
    pub failure: Option<MediaFailure>,
    /// 逐项条目（一次最多 100 条，多的用 cursor 翻）
    pub items: Vec<ItemView>,
    pub items_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ItemView { pub item_id: String, pub name: String, pub state: ItemState }

/// 两本账各一项，壳才知道去清哪边
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct QuotaView { pub provider_upload: QuotaEntry, pub device_offline: QuotaEntry }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct QuotaEntry { pub used: u64, pub limit: Option<u64> }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackViewModel { pub session: PlaybackSession, pub queue: Queue, pub plan: Option<PlaybackPlan> }

## 能力表 `media_source`（只加不改）

在 `crates/wonton-core/src/plugin/capabilities.rs:120-128` 那行 `personal_data` 旁边加一行。**走 `provides.capabilities` 这张表，不开专用顶级字段**——选路、凭据、安装确认、`spec_floor` 都复用现成机制。

| 字段 | 值 |
| --- | --- |
| 能力名 | `"media_source"`（`wonton-protocol::media::MEDIA_CAPABILITY`） |
| `credentials` | `Required`（能力表这一侧）；对清单侧的要求是 `credentials` 必须有 `issue`/`revoke`（`capabilities.rs:52`、`provides.rs:67-73`） |
| `interface` | 一句契约描述（能力表字段，不是清单字段） |
| `hosted` | `None`（`personal_data` 也是 `None`；`HostedForm` 只给核心托管的 static_web/remote_mcp） |
| 高权限 | 否 |
| 核心侧 | 使用方客户端 `plugin/media_source.rs` |
| 契约测试 | 两个假提供方，照 `plugin/tests/personal_data_contract/mod.rs:1-10` |

与 `jellyfin` 能力并存：`jellyfin` 给 Seerr 之类窄用途，`media_source` 管播放与传输，同一个插件可以两样都提供，Seerr 不迁。

## 清单 `provides.capabilities.media_source`（只加不改）

在现有的 `ProviderService`（`crates/wonton-protocol/src/manifest/provides.rs:16-31`）上加两个可选数组，取值受枚举约束：

```rust
// provides.rs，ProviderService 内
/// 自报能做的操作；核心仍以 /_wonton/capabilities 为准，这里给安装预检
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub operations: Vec<MediaOperation>,
/// 自报的内容种类
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub kinds: Vec<MediaKind>,
```

```rust
// wonton-protocol/src/media/mod.rs（枚举只在这里定义一次，别处引用）
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind { Photo, Video, Audio }

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MediaOperation { Upload, Download, Playback, List }
```

校验：`media_source` 出现时 `operations`、`kinds` 都不许空，`credentials` 必须有 `issue` 与 `revoke`（清单形状是 `{issue, revoke}`，没有 `required` 键）；`service` 必须在本插件 `services` 里。`spec_floor.rs` 登记用到这些字段的最低 `spec`。

## `plugin-spec.md` 要补的章节（只加不改）

| 章节 | 写什么 |
| --- | --- |
| “能力”表加 `media_source` 一行 | 名字、传输 HTTP、凭据 Required、非高权限、操作自报 |
| 新节“媒体源：清单声明” | `provides.capabilities.media_source` 形状，`service`/`operations`/`kinds`/`credentials` 逐字段说明 |
| 新节“媒体源：路径” | 正文“媒体源契约”表那 11 行原样搬进去，含 `X-Wonton-User` 语义 |
| 新节“媒体源：错误信封” | `MediaFailure{code,message,retry_after?}`，码就是正文失败类别表第一列的十个 |
| 新节“媒体源：适配器进程” | 边车容器或插件主体内的一段都行，按 `service` 连 |
| 新节“媒体源：契约测试” | 两个假提供方的跑法，指向 `plugin/tests/media_source_contract/` |

`:1577-1700` 那段现有的能力写法当模板，不改旧句。

## 核心 HTTP API `/_wonton/api/media/*`（只加不改）

全走现有网关鉴权；错误体统一 `MediaFailure` 信封。

| 路径 | 方法 | 请求体 | 响应体 |
| --- | --- | --- | --- |
| `/tasks` | `POST` | `TaskCreate{kind, person, device, target, options}` | `Task` |
| `/tasks` | `GET` | 无（query: `person?`, `kind?`, `state?`） | `{tasks: Vec<Task>, cursor?}` |
| `/tasks/{id}` | `GET` | 无 | `Task` |
| `/tasks/{id}/pause` | `POST` | `{reason: PauseReason}` | `Task` |
| `/tasks/{id}/resume` | `POST` | 无 | `Task` |
| `/tasks/{id}/cancel` | `POST` | 无 | `Task` |
| `/tasks/{id}/retry` | `POST` | 无（终态才允许，设 `retry_of`） | `Task` |
| `/sessions` | `POST` | `PlaybackRequest`（含 `position_ms`、`person`、`device`、`profile`） | `PlaybackSession`（`plan` 已填） |
| `/sessions` | `GET` | 无（query: `person?`） | `{sessions: Vec<PlaybackSession>}` |
| `/sessions/{id}` | `GET` | 无 | `PlaybackSession` |
| `/sessions/{id}/control` | `POST` | `MediaAction` | `PlaybackSession` |
| `/sessions/{id}/progress` | `POST` | `{position_ms, wall_clock}` | `204` |
| `/sessions/{id}/stop` | `POST` | `{position_ms, reason}` | `204` |
| `/queue` | `GET` | 无（query: `person`） | `Queue` |
| `/queue` | `POST` | `QueueOp{revision, op}` | `Queue`；`revision` 不相等回 `409` + 当前 `Queue` |
| `/queue` | `PUT` | 整条 `Queue`（核心推给提供方的 `savePlayQueue` 翻法，带 `position`） | `204` |
| `/tasks/{id}/uploads/{item}` | `POST`/`PATCH` | 原始字节；转发提供方 `POST /uploads`、`PATCH /uploads/{upload_id}` |
| `/tasks/{id}/items/{item}/content` | `GET` | 原始字节；转发 `/items/{id}/download`，`Range` 原样 |
| `/sessions/{id}/stream` | `GET` | 原始字节；转发 plan 的流地址 |
| `/sessions/{id}/cast-url` | `POST` | `{target: "airplay"\|"cast"}` | `{url, expires_at}`（绑 LAN 的一次性绝对地址） |
| `/commands` | `GET` | 无（query: `device`, `after: seq`） | 长轮询；`{commands: Vec<DeviceCommand>}`，超时 `204` |
| `/commands/{id}/ack` | `POST` | `{seq, ok, failure?}` | `204`（幂等） |

命令体是 `DeviceCommand` 与 `MediaAction`，类型定义在上面 `media/view.rs` 一处，这里不重复。

`/streams` 不在服务端 API 里：是客户端核本机的回环（见 ADR 正文第 2 节）。服务端只给 `POST /sessions/{id}/cast-url`，回一个绑 LAN 的一次性绝对地址。

## 客户端核导出（只加不改）

```rust
// wonton-client 对外动作
pub enum Action { /* …既有… */ , Media(MediaAction) }

pub fn subscribe_media(&self, cb: Callback<MediaViewModel>);
pub fn subscribe_playback(&self, cb: Callback<PlaybackViewModel>);
pub fn player_state(&self) -> PlayerState; // Playing/Paused/Buffering/Stopped + position
/// 本机回环前缀（播放器与上传/下载只连这里），带随机令牌
pub fn media_loopback(&self) -> String;
```

`MediaViewModel` / `PlaybackViewModel` 的字段进 `wonton-protocol::media`，形状照 `file/mod.rs:291-320` 的 ViewModel 写法；`player_state` 只在壳里用，不进协议。

## 头与 `_meta`（只加不改）

| 名 | 方向 | 值 |
| --- | --- | --- |
| `X-Wonton-User`（头） | 核心 → 提供方 | 任务或会话的主人（复用 `personal_data` 的那个常量） |
| `X-Wonton-Device`（头） | 客户端核 → 核心 | 设备标识；同一个人两台设备时区分 |
| `app.wonton/speakers`（`_meta`） | 核心 → 插件 | 本轮说话人数组，定义在 ADR-0114 第 2 项，这里只引用 |

## MCP 工具与参数（只加不改）

| 工具 | 参数 | 返回 |
| --- | --- | --- |
| `wonton__media_status` | `{kind?, person?}` | 任务与会话列表，带进度与失败原因 |
| `wonton__backup_now` | `{device?, kinds?}` | `Task` |
| `wonton__media_download` | `{device, item, kind}` | `Task` |
| `wonton__media_play` | `{device?, item, kind, person?}` | `PlaybackSession` |
| `wonton__media_control` | `{session, ctl: ControlOp}` | `PlaybackSession` |
| `wonton__media_queue` | `{op: QueueOp\|read, revision?}` | `Queue` |
| `wonton__media_sessions` | `{person?}` | 这个人的全部设备与会话能力 |
| `wonton__media_task` | `{task, op: pause\|resume\|cancel\|retry}` | `Task` |
| `wonton__media_delete` | `{device, items: Vec<MediaItemRef>}` | `204` |

`person` 只能填本轮 `speakers` 里的人；共享 Bot 省略则取唯一说话人，多人必填。

## 事件名与载荷（只加不改）

| 事件 | 载荷 |
| --- | --- |
| `media.task.progress`（只在线的推，不进队列） | `{task, done, total, unit}` |
| `media.task.done` | `{task, files?: Vec<DoneItem>}` |
| `media.task.failed` | `{task, failure: MediaFailure}` |
| `media.playback.started` | `{session, device, item}` |
| `media.playback.finished` | `{session, position_ms, reason}` |
| `media.session.taken_over` | `{session, by_device}` |

投递、按人通知、叫醒照 ADR-0114 第 3 项与第 4 项，这里只定名字与载荷。

## 这个 PR 不做的

- 不发 `media_source` 之外的任何能力，不改 `personal_data` 契约。
- 不写核心侧使用方实现（`plugin/media_source.rs`）、不写假提供方与契约测试（下一张卡）。
- 不改 `PageMediaOwner.kt:42-53` / `PageMediaActive.swift`，网页 `<audio>` 的收口归实现卡（M22）。
- 不改 `jellyfin` 能力与 Seerr。
- 不填离线下载配额默认值。
- 不动 `plugin-spec.md` 里除上表五节之外的任何章；不碰 `provides` 以外的清单字段。
- 事件载荷里不重复 ADR-0114 的说话人/通知/深链字段，只引用。