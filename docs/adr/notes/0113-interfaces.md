# ADR-0113 接口清单（前半：`wonton-protocol` 的 `media` 模块）

只改接口的 PR 的全部改动。**全部只加不改**，没有破坏性项；ADR 正文列了为什么不上破坏性改动（#585 的收口在壳内）。字段级照 ADR-0113 第 3、4 节，类型用 Rust 写。

模块位置：`crates/wonton-protocol/src/media/`，`lib.rs` 加 `pub mod media;`，按职责分 `mod.rs`（常量与入口类型）、`task.rs`、`session.rs`、`view.rs`。

## `media/mod.rs`

```rust
/// 能力名，插件清单 `provides.media_source` 与能力表共用这一串。
pub const MEDIA_CAPABILITY: &str = "media_source";
/// 每个请求都带；值是本机用户名（ADR-0088 的按人身份）。
pub const WONTON_USER_HEADER: &str = "X-Wonton-User";

pub const PATH_CAPABILITIES: &str = "/_wonton/capabilities";
pub const PATH_UPLOADS: &str = "/uploads";
pub const PATH_PLAYBACK: &str = "/playback";
pub const PATH_ITEMS: &str = "/items";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind { Photo, Video, Audio }

/// 提供方自报能做的事；核心据它关掉不支持的按钮，不拿它当能力协商。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MediaOperation { Upload, Download, Playback, List }

/// 一条媒体条目的不变量，提供方内部 id 与呈给用户的字段分开。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MediaItemRef {
    pub id: String,          // 提供方内部 id，只回传给提供方
    pub kind: MediaKind,
    pub title: String,
    pub duration_ms: Option<u64>,
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
pub enum TaskKind { Upload, Download, Playback }



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
    AuthenticationRequired,
    PermissionDenied,
    UnsupportedByDevice,
    RateLimited,
    MediaGone,
    Network,
    Server,
    StorageFull,
    SuspendedBySystem,
    DeviceNoResponse,
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
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TaskTarget {
    Upload(String),          // 条目相对路径，归提供方解释
    Download(MediaItemRef),
    Playback(MediaItemRef),
}

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
pub struct Queue { pub revision: Revision, pub entries: Vec<MediaItemRef>, pub cursor: Option<usize> }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackRequest {
    pub item: MediaItemRef,
    pub device: DeviceId,
    pub profile: DeviceProfile,
}

/// 提供方给的核心可播计划：直连或转码流，以及过期时间。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackPlan {
    pub session: PlaybackSessionId,
    pub stream: ProviderStream,
    pub audio: Vec<MediaTrack>,
    pub seekable: bool,
    pub expires_at: Timestamp,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ProviderStream { Direct(String), Transcoded(String) }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MediaTrack { pub index: u16, pub language: String, pub label: Option<String> }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackSession {
    pub id: PlaybackSessionId,
    pub person: Person,
    pub device: DeviceId,
    pub item: MediaItemRef,
    pub state: TaskState,
    pub position_ms: u64,
}

/// 建任务用的入参：`options` 是各轴自己的开关（只在传输、只在充电、转码率上限）。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskCreate { kind: TaskKind, person: Person, device: DeviceId, target: TaskTarget, options: TaskOptions }

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TaskOptions { pub wifi_only: bool, pub charging_only: bool, pub max_bitrate: Option<u32> }

## `media/view.rs`

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ControlOp { Play, Pause, Resume, SeekMs(u64), Next, Prev, Volume(u8) }

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum StopReason { User, Agent, TakenOver, Finished, Error }

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum QueueOp { Read, Enqueue { at: Option<usize>, item: MediaItemRef }, Remove { index: usize }, Jump { index: usize }, Clear }

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
pub struct TaskView { pub id: TaskId, pub kind: TaskKind, pub title: String, pub state: TaskState, pub progress: Progress, pub failure: Option<MediaFailure> }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct QuotaView { pub used_bytes: u64, pub limit_bytes: Option<u64> }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackViewModel { pub session: PlaybackSession, pub queue: Queue, pub plan: Option<PlaybackPlan> }

## 能力表 `media_source`（只加不改）

在 `crates/wonton-core/src/plugin/capabilities.rs:120-128` 那行 `personal_data` 旁边加一行，形状照它：

| 字段 | 值 | 与 `personal_data` 同不同 |
| --- | --- | --- |
| 能力名 | `"media_source"`（`wonton-protocol::media::MEDIA_CAPABILITY`） | 同（核心定名字） |
| `Credentials` | `Required` | 同 |
| 高权限 | 否 | 同 |
| 提供方要求 | 提供方对使用方（核心）签一把能力凭据；每个请求带 `X-Wonton-User` | 同 |
| 核心侧 | 有使用方客户端（`plugin/media_source.rs`） | 同 |
| 契约测试 | 两个假提供方，照 `plugin/tests/personal_data_contract/mod.rs:1-10` | 同（用例不同，见 ADR 正文“契约测试”段） |
| 操作集 | 由 `GET /_wonton/capabilities` 自报，核心据它关掉不支持的按钮 | 新 |

标“只加不改”：能力表是常量表，加一行不影响既有能力。

## 清单 `provides.media_source`（只加不改）

`crates/wonton-protocol/src/manifest/provides.rs:21-31` 的 `provides` 加一个可选字段；没有它就等于这个插件不是媒体源。

```rust
// wonton-protocol/src/manifest/provides.rs，加在 Provides 里
#[serde(default, skip_serializing_if = "Option::is_none")]
pub media_source: Option<MediaSourceDecl>,
```

```rust
// wonton-protocol/src/manifest/media_source.rs
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct MediaSourceDecl {
    /// 适配器在插件内的端口；核心按插件地址 + 这个端口连
    pub service: String,
    pub port: u16,
    /// 自报的操作集；核心仍以 /_wonton/capabilities 为准，这里只是安装时预检
    pub operations: Vec<MediaOperation>,
    /// 自报的内容种类
    pub kinds: Vec<MediaKind>,
    /// 凭据怎么来；第一版只有 "required"
    pub credentials: MediaCredentials,
}

// wonton-protocol/src/media/mod.rs
pub enum MediaOperation { Upload, Download, Playback, List }
pub enum MediaKind { Photo, Video, Audio }
pub enum MediaCredentials { Required }
```

校验照 `plugin/capabilities.rs` 的既有做法：`operations` 空、`kinds` 空、端口 0 都算清单非法。

## `plugin-spec.md` 要补的章节（只加不改）

| 章节 | 写什么 |
| --- | --- |
| “能力”表加 `media_source` 一行 | 名字、传输 HTTP、凭据 Required、非高权限、操作自报 |
| 新节“媒体源：清单声明” | 上面那个 JSON 形状，逐字段必填/选填 |
| 新节“媒体源：路径” | 正文“媒体源契约”表那 11 行原样搬进去，含 `X-Wonton-User` 语义 |
| 新节“媒体源：错误信封” | `MediaFailure{code,message,retry_after?}`，码就是正文失败类别表第一列的十个 |
| 新节“媒体源：适配器进程” | 边车容器或页内端口二选一，端口写进 `provides.media_source.port` |
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
| `/sessions` | `POST` | `PlaybackRequest{person, device, item, profile}` | `PlaybackSession`（返回时已含 `PlaybackPlan`） |
| `/sessions` | `GET` | 无（query: `person?`） | `{sessions: Vec<PlaybackSession>}` |
| `/sessions/{id}` | `GET` | 无 | `PlaybackSession` |
| `/sessions/{id}/control` | `POST` | `MediaAction` | `PlaybackSession` |
| `/sessions/{id}/progress` | `POST` | `{position_ms, wall_clock}` | `204` |
| `/sessions/{id}/stop` | `POST` | `{position_ms, reason}` | `204` |
| `/queue` | `GET` | 无（query: `person`） | `Queue` |
| `/queue` | `POST` | `QueueOp{revision, op}` | `Queue`；`revision` 不相等回 `409` + 当前 `Queue` |
| `/streams/{session}` | `GET` | 无 | `302` 到回环令牌流地址（照 `stream.rs:1-8`） |
| `/commands` | `GET` | 无（query: `device`, `after: seq`） | 长轮询；`{commands: Vec<DeviceCommand>}`，超时 `204` |
| `/commands/{id}/ack` | `POST` | `{seq, ok, failure?}` | `204`（幂等） |

设备命令体：

```rust
pub struct DeviceCommand { pub command_id: Uuid, pub seq: u64, pub body: MediaAction }
pub enum MediaAction {
    StartUpload { task: TaskId }, StartDownload { task: TaskId },
    Play { session: PlaybackSessionId }, Control { session: PlaybackSessionId, ctl: ControlOp },
    Stop { session: PlaybackSessionId, reason: StopReason },
}
```

## 客户端核导出（只加不改）

```rust
// wonton-client 对外动作
pub enum Action { /* …既有… */ , Media(MediaAction) }

pub fn subscribe_media(&self, cb: Callback<MediaViewModel>);
pub fn subscribe_playback(&self, cb: Callback<PlaybackViewModel>);
pub fn player_state(&self) -> PlayerState; // Playing/Paused/Buffering/Stopped + position
```

`MediaViewModel` / `PlaybackViewModel` 的字段进 `wonton-protocol::media`，形状照 `file/mod.rs:291-320` 的 ViewModel 写法；`player_state` 只在壳里用，不进协议。

## `_meta` 键（只加不改）

| 键 | 方向 | 值 |
| --- | --- | --- |
| `app.wonton/device` | 壳 → 核心 | 设备稳定标识 |
| `app.wonton/speaker` | 核心 → 插件 | 本轮说话人（引用 ADR-0114 第 2 项，不在这里定字段） |

`X-Wonton-User` 是头，不是 `_meta` 键。

## MCP 工具与参数（只加不改）

| 工具 | 参数 | 返回 |
| --- | --- | --- |
| `wonton__media_status` | `{kind?}` | 任务与会话列表，带进度与失败原因 |
| `wonton__backup_now` | `{device?, kinds?}` | `Task` |
| `wonton__media_download` | `{device, item, kind}` | `Task` |
| `wonton__media_play` | `{device?, item, kind}` | `PlaybackSession` |
| `wonton__media_control` | `{session, ctl: ControlOp}` | `PlaybackSession` |
| `wonton__media_queue` | `{op: QueueOp\|read, revision?}` | `Queue` |
| `wonton__media_sessions` | 无 | 这个人的全部设备与会话能力 |

## 事件名与载荷（只加不改）

| 事件 | 载荷 |
| --- | --- |
| `media.task.progress` | `{task, done, total, unit}` |
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
- 不填离线下载配额默认值。
- 不动 `plugin-spec.md` 里除上表五节之外的任何章；不碰 `provides` 以外的清单字段。
- 事件载荷里不重复 ADR-0114 的说话人/通知/深链字段，只引用。