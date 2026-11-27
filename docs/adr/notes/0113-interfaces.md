# ADR-0113 接口清单（前半：`wonton-protocol` 的 `media` 模块）

只改接口的 PR 的全部改动。**全部只加不改**，没有破坏性项；ADR 正文列了为什么不上破坏性改动（#585 的收口在壳内）。字段级照 ADR-0113 第 3、4 节，类型用 Rust 写。

所有要过壳的类型都照 `file/mod.rs:291-320` 的写法加 `#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]`（枚举加 `uniffi::Enum`）；下面为了可读没有逐条重复。不过壳的 `IllegalTransition`、`Backoff`、`MediaOperation` 不加（`IllegalTransition` 将来要过壳得先改成 `uniffi::Error`）。

取值不认识时怎么办（AGENTS.md）：除了壳要执行的 `QueueOp`，一律不报错，带 `Other(String)` 把原文存进去、原样序列化回去。
核心和 App 分开升级，新核发的新命令旧 App 跳过那一条，同批其他命令照常读；新 App 发的新字段旧核也不会整条拒。

- 服务端 → 消费方的取值枚举（`MediaKind`、`TaskKind`、`TaskState`、`PauseReason`、`MediaFailureCode`、`ItemState`、`TrackKind`、`SegmentKind`、`ProviderStream`）原本就这样；
- 核 → 设备/App 下发的 `ControlOp`、`StopReason`、`MediaAction`（`DeviceCommand.body`）、`QueueMode`、`TaskTarget`、`ProgressUnit` 和清单/协商面的 `MediaOperation` 同样落 `Other`，载荷丢弃只留标签名；`MediaOperation` 落 `Other` 后安装预检忽略它，不再让插件装不上；
- `QueueOp` 是壳要执行的动作，认不得就没法执行，仍报错。
- 未知 `TaskState` 的转移规则只有服务端知道：只能进它，不能从它出来；字符串写法的原值原样回写，对象写法（暂停原因、失败体）只留标签名、载荷丢弃，读成 `"migrating"`；
- `ItemState` 带内部标记，未知状态的原文就放在 `state` 里（`{"state":"frozen"}`），未知的附加键忽略。

模块位置：`crates/wonton-protocol/src/media/`，`lib.rs` 加 `pub mod media;`，按职责分 `mod.rs`（常量、路径拼接与入口类型）、`task.rs`、`session.rs`、`view.rs`。

## `media/mod.rs`

```rust
/// 能力名，插件清单 `provides.capabilities.media_source` 与能力表共用这一串。
pub const MEDIA_CAPABILITY: &str = "media_source";
/// 复用 `personal_data` 已有的那一个（`personal_data.rs:22`），不另定义。
pub use crate::personal_data::WONTON_USER_HEADER;
/// 同一个人两台设备时区分；不打用户表。
pub const WONTON_DEVICE_HEADER: &str = "X-Wonton-Device";

/// Unix 毫秒（UTC）。
pub type Timestamp = u64;

pub const PATH_CAPABILITIES: &str = "/_wonton/capabilities";
pub const PATH_UPLOADS: &str = "/uploads";
pub const PATH_PLAYBACK: &str = "/playback";
pub const PATH_ITEMS: &str = "/items";

/// 列条目的路径，`query` 原样接在后面，不转义。
pub fn items_path(query: &str) -> String { format!("{PATH_ITEMS}?{query}") }

/// 条目自己的路径，`id` 原样接在后面，不转义。
pub fn item_path(id: &str) -> String { format!("{PATH_ITEMS}/{id}") }

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
    pub source: Option<String>, // 给出这一条的媒体源插件 id，核心填；见“选哪个提供方”
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

#[derive(Debug, Clone, PartialEq)]
pub enum TaskKind { Upload, Download, Other(String) }



#[derive(Debug, Clone, PartialEq)]
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
    Other(String),
}

impl TaskState {
    pub fn as_str(&self) -> &str;
    fn is_terminal(&self) -> bool;  // Done | Partial | Failed | Cancelled
    pub fn transition(self, to: Self) -> Result<Self, IllegalTransition>;
}

/// 「从，到」直接转，规则同 `transition`。
impl TryFrom<(TaskState, TaskState)> for TaskState { type Error = IllegalTransition; }

// 合法转移（ADR 第 3 节状态图）：
// Queued → Preparing | Paused | WaitingNetwork | WaitingPower | Cancelled（排队中也能暂停、等条件、直接取消）
// Preparing → Running | Done | Partial | Failed | Cancelled
// Running → Paused | WaitingNetwork | WaitingPower | Done | Partial | Failed | Cancelled
// Paused | WaitingNetwork | WaitingPower → Queued | Running | Paused（换原因） | WaitingNetwork | WaitingPower | Failed | Cancelled
// 非终态 → Other，Other → Other；终态不可逆；其余（含 Any → Preparing、Queued → Running、终态 → Queued）非法。
// 条件恢复和到点重试都走 `resume`，回 `Queued`；`due(at)` 只列到点的 `WaitingNetwork` 任务，不认领。

#[derive(Debug, Clone, PartialEq)]
pub enum PauseReason { User, Agent, LowBattery, SuspendedBySystem, Other(String) }

/// 失败的类别就是协议面，不是文案；壳按类别出字。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MediaFailure {
    pub code: MediaFailureCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
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
    SourceRequired, // 见“选哪个提供方”
    StaleRevision,  // 队列改动撞上别处先改，见第 4 节
    Other(String),
}

impl MediaFailureCode {
    /// 这类失败该等多久再退避。`Other` 按不退避算，交给壳显示原文。
    pub fn backoff(&self) -> Backoff;
    /// 这类失败是不是等等就可能自己好；`SystemPause` 与不退避的都算不是。
    pub fn backoff_is_retryable(&self) -> bool;
}

/// 一个失败类别的退避档。退避由核心算，不过壳。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Backoff { None, Once, Standard, ThreeTimes, RetryAfter, SystemPause }

/// 状态机不允许的转移。只在 Rust 的 `TryFrom` 里用，不过壳。
#[derive(Debug, Clone, PartialEq)]
pub struct IllegalTransition { pub from: TaskState, pub to: TaskState }

/// 一条长任务。上传与下载共用；`target` 与 `progress.unit` 各自成立。
/// 名字带 `Media`：Swift 绑定在全局命名空间，`Task` 会和 Swift 的 `Task` 撞。JSON 形状不受影响。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MediaTask {
    pub id: TaskId,
    pub kind: TaskKind,
    pub person: Person,
    pub device: DeviceId,
    pub target: TaskTarget,
    pub state: TaskState,
    pub progress: Progress,
    pub retry_count: u8,
    pub retry_of: Option<TaskId>,
    /// 下一次自动重试的秒级 Unix 时间；只在 `WaitingNetwork` 时有值，由核心算。
    pub next_attempt_at: Option<u64>,
    pub created_at: Timestamp,
    /// 只在上传里有；逐项失败原文
    pub partial_failures: Vec<FailedItem>,
}

#[derive(Debug, Clone)]
pub enum TaskTarget {
    Upload(Vec<UploadItem>), // 一批文件；相册备份 = 一个任务 + N 个 item
    Download(MediaItemRef),
    Other(String), // 新目标，载荷丢弃
}

// 内部标记放不下序列和结构，载荷另起 `value` 键：{"kind":"upload","value":[…]}
// 序列化不带 `content` 会报 `cannot serialize tagged newtype variant … containing a sequence`。

/// 上传的一项：路径、大小、校验和、拍摄时间，都是提供方要的。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UploadItem {
    pub item_id: String,
    pub path: String,
    pub size: u64,
    pub checksum: Option<String>,
    pub taken_at: Option<Timestamp>,
    pub state: ItemState,
    /// 服务端已知的已收字节；非 Done 项有效
    pub upload_offset: u64,
}

/// 逐项状态；`Partial` 由这里聚出来，不靠猜。未知状态把原文放进 `state`（`{"state":"frozen"}`）。
#[derive(Debug, Clone, PartialEq)]
pub enum ItemState { Queued, Running, Done, Skipped, Failed(MediaFailure), Other(String) }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FailedItem { pub item_id: String, pub path: String, pub failure: MediaFailure }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Progress { pub done: u64, pub total: Option<u64>, pub unit: ProgressUnit }

#[derive(Debug, Clone)]
pub enum ProgressUnit { Bytes, Items, Other(String) }

/// 建任务用的入参：`options` 是各轴自己的开关（只在传输、只在充电、转码率上限）。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskCreate { pub kind: TaskKind, pub person: Person, pub device: DeviceId, pub target: TaskTarget, pub options: TaskOptions }

fn one() -> u8 { 1 }

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TaskOptions {
    pub wifi_only: bool,
    pub charging_only: bool,
    pub max_bitrate: Option<u32>,
    /// 相册范围（可选）；只传相册不传 items 时由设备枚举
    pub albums: Vec<String>,
    /// 同时在跑几项，1–4
    #[serde(default = "one")]
    pub concurrency: u8, // 1–4，缺省 1
    /// 立即备份：忽略 wifi/充电条件一次
    pub force_now: bool,
    pub source: Option<String>, // 上传传到哪一家；空着按“选哪个提供方”
}

// `Default` 手写（`concurrency: 1`）：derive 出来的 `concurrency` 是 0，越界；serde 缺省走 `one()`。
impl Default for TaskOptions { fn default() -> Self; }

## `media/session.rs`

```rust
pub type PlaybackSessionId = Uuid;
pub type Revision = u64;

/// 队列自带 revision；所有改动带读到的 revision，不等就 409。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Queue { pub revision: Revision, pub entries: Vec<MediaItemRef>, pub cursor: Option<usize>, pub mode: QueueMode, pub sleep_at_ms: Option<u64> }

impl Queue {
    /// 队列改了就加一版（`revision + 1`，进位时回绕），让并发的旧版本落空。
    pub fn bump(&mut self);
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueueMode { Sequential, Shuffle, RepeatAll, RepeatOne, Other(String) }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackRequest {
    /// 从上次位置起播；0 表示从头
    pub position_ms: u64,
    pub audio: Option<u16>,
    pub subtitle: Option<u16>,
    pub item: MediaItemRef,
    pub device: DeviceId,
    pub person: Person,
    pub profile: DeviceProfile,
}

/// 提供方给的核心可播计划。`session` 是上游会话号，和核心的 `PlaybackSessionId` 是两套，映射只在核心。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackPlan {
    pub session: String,
    /// 核心填好，客户端只读这个
    pub stream: ProviderStream,
    /// 提供方原址，只给 cast-url 用
    pub origin_stream: Option<ProviderStream>,
    pub segments: Vec<Segment>,
    pub audio: Vec<MediaTrack>,
    pub subtitles: Vec<MediaTrack>,
    pub seekable: bool,
    pub duration_ms: u64,
    pub expires_at: Timestamp,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStream { Direct(String), Transcoded(String), Other(String) }

// 外部标记，标签是小写：{"direct":"https://…"}、{"transcoded":"https://…"}。未知流型的载荷是字符串时
// 存进 `Other`（{"other":"https://…"}），否则只留标签名（{"other":"opaque"}）；新流型（如 dash）来了不判死整条 `PlaybackPlan`。

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MediaTrack { pub index: u16, pub language: String, pub label: Option<String>, pub kind: TrackKind, pub selected: bool }

#[derive(Debug, Clone, PartialEq)]
pub enum TrackKind { Audio, Subtitle, Other(String) }

/// 章节、片头片尾。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Segment { pub kind: SegmentKind, pub start_ms: u64, pub end_ms: u64 }

#[derive(Debug, Clone, PartialEq)]
pub enum SegmentKind { Chapter, Intro, Outro, Other(String) }

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
    /// 停的原因，停了才有且不再变
    pub stop_reason: Option<StopReason>,
}

## `media/view.rs`

```rust
#[derive(Debug, Clone)]
pub enum ControlOp { Play, Pause, Resume, SeekMs(u64), Next, Prev, Volume(u8), SetAudio(u16), SetSubtitle(Option<u16>), Other(String) }

#[derive(Debug, Clone)]
pub enum StopReason { User, Agent, TakenOver, Finished, Error, Removed, Other(String) }

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", tag = "op", content = "value")]
pub enum QueueOp { Read, Enqueue { at: Option<usize>, item: MediaItemRef }, Remove { index: usize }, Jump { index: usize }, Clear, SetMode(QueueMode) }

// 同 `TaskTarget`，载荷另起 `value`：{"op":"set_mode","value":"shuffle"}；`Read` 等无载荷的没有 `value`。

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DoneItem { pub item: MediaItemRef, pub local_path: Option<String> }

/// 壳→核的动作与核→设备的命令是同一个类型。
#[derive(Debug, Clone)]
pub enum MediaAction {
    StartUpload { task: TaskId },
    StartDownload { task: TaskId },
    Playback { session: PlaybackSessionId },
    Control { session: PlaybackSessionId, ctl: ControlOp },
    Stop { session: PlaybackSessionId, reason: StopReason },
    Queue { person: Person, revision: Revision, op: QueueOp },
    Other(String), // 新动作，旧壳跳过这一条、同批其他照常执行
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
pub struct ItemView { pub item_id: String, pub name: String, pub state: ItemState, pub upload_offset: u64 }

/// 两本账各一项，壳才知道去清哪边
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct QuotaView { pub provider_upload: QuotaEntry, pub device_offline: QuotaEntry }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct QuotaEntry { pub used: u64, pub limit: Option<u64> }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlaybackViewModel { pub session: PlaybackSession, pub queue: Queue, pub plan: Option<PlaybackPlan> }

## 服务端核的 `SessionService`（M9，不过壳）

`crates/wonton-core/src/media/session_service.rs`，事实源在本机 SQLite；HTTP、事件与提供方调用在外面。

```rust
pub struct SessionService { /* Store、毫秒时钟、位置节流基准 */ }

pub enum SessionServiceError {
    Store(MediaStoreError),              // 含 `SessionNotFound`
    Transition(IllegalTransition),       // 已停的会话收到 `control`
    ConcurrentChange { session: PlaybackSessionId },
}

impl SessionService {
    pub fn start(&self, request: &PlaybackRequest, plan: PlaybackPlan)
        -> Result<(PlaybackSession, Option<PlaybackSessionId>), SessionServiceError>;
    pub fn control(&self, session: PlaybackSessionId, ctl: ControlOp)
        -> Result<PlaybackSession, SessionServiceError>;
    pub fn progress(&self, session: PlaybackSessionId, position_ms: u64, wall_clock: u64)
        -> Result<bool, SessionServiceError>;   // false = 被节流或会话已停
    pub fn stop(&self, session: PlaybackSessionId, position_ms: u64, reason: StopReason)
        -> Result<PlaybackSession, SessionServiceError>;
    pub fn active(&self, person: &str) -> Result<Option<PlaybackSession>, SessionServiceError>;
}
```

- 第二个返回值是被接管的旧会话号，调用方拿它发 `media.session.taken_over`；旧会话的位置保留它最后一次上报的值，不被新请求的起点覆盖。新会话的起播位置夹到 `plan.duration_ms`。
- 起播、控制、位置、停止都按 `sessions.version` 比较落库，落不上重读；库里`(person) WHERE stop_reason IS NULL`的唯一索引兜住并发起播，后到的接管先到的。只把唯一约束的失败认作“已有活动会话”，主键撞等其他约束错原样报 `Database`；迁移 v4 建索引前会把同一个人多余的活动会话置成 `taken_over` 终态、只留最新一条。
- `progress` 同一会话两次落库至少隔 5 秒（`wall_clock` 与服务的钟同源，都是 Unix 毫秒），`pause`/`seek`/`stop` 立即落并把节流基准推到现在；已停的会话上报回 `false` 不落。被别处改掉时重读一次再判，仍落不上也回 `false`，不报 `ConcurrentChange`。
- `stop` 幂等：再停只把位置往后推，位置变小保留旧值；原因与终态保留第一次的。`Finished` 落 `Done`、`Error` 落 `Failed{upstream_error}`、其余落 `Cancelled`。
- `Next`/`Prev` 按该人的队列移动 `cursor` 并换 `session.item`、位置归零：核心先算出换到哪一项、向它的来源要到计划，再把会话、队列指针、计划和会话记的提供方一起比较写，中间有别的改动就重读重来；要不到计划就什么都不写，会话还在原来那一项、按旧计划放；到头不回绕，`repeat_all` 往后回第一首，队列为空不动。`SetAudio`/`SetSubtitle` 只改计划里的 `selected`，重取计划归 M11。`Volume` 当前没有字段，只接受不改。

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
/// 内容的种类。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MediaKind { Photo, Video, Audio, Other(String) }

/// 请求/协商面：认不得的操作落 `Other`，安装预检忽略。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MediaOperation { Upload, Download, Playback, List, Other(String) }
```

校验：`media_source` 出现时 `operations`、`kinds` 都不许空，`credentials` 必须有 `issue` 与 `revoke`（清单形状是 `{issue, revoke}`，没有 `required` 键）；`service` 必须在本插件 `services` 里。`spec_floor.rs` 登记用到这些字段的最低 `spec`。

## `plugin-spec.md` 要补的章节（只加不改）

| 章节 | 写什么 |
| --- | --- |
| “能力”表加 `media_source` 一行 | 名字、传输 HTTP、凭据 Required、非高权限、操作自报 |
| 新节“媒体源：清单声明” | `provides.capabilities.media_source` 形状，`service`/`operations`/`kinds`/`credentials` 逐字段说明 |
| 新节“媒体源：路径” | 正文“媒体源契约”表那 14 行原样搬进去，含 `X-Wonton-User` 语义 |
| 新节“媒体源：错误信封” | `MediaFailure{code,message,retry_after?}`，码就是正文失败类别表第一列的十五个 |
| 新节“媒体源：适配器进程” | 边车容器或插件主体内的一段都行，按 `service` 连 |
| 新节“媒体源：契约测试” | 两个假提供方的跑法，指向 `plugin/tests/media_source_contract/` |

`:1577-1700` 那段现有的能力写法当模板，不改旧句。

## 核心 HTTP API `/_wonton/api/media/*`（只加不改）

全走现有网关鉴权；错误体统一 `MediaFailure` 信封。

| 路径 | 方法 | 请求体 | 响应体 |
| --- | --- | --- | --- |
| `/tasks` | `POST` | `TaskCreate{kind, person, device, target, options}` | `MediaTask` |
| `/tasks` | `GET` | 无（query: `person?`, `kind?`, `state?`） | `{tasks: Vec<MediaTask>, cursor?}` |
| `/tasks/{id}` | `GET` | 无 | `MediaTask`（上传的每项带当前 `upload_offset`，重连后靠它续） |
| `/tasks/{id}/pause` | `POST` | `{reason: PauseReason}` | `MediaTask` |
| `/tasks/{id}/resume` | `POST` | 无 | `MediaTask` |
| `/tasks/{id}/cancel` | `POST` | 无 | `MediaTask` |
| `/tasks/{id}/retry` | `POST` | 无（终态才允许，设 `retry_of`） | `MediaTask` |
| `/tasks/{id}/progress` | `POST` | `{done, total?}`（下载由设备报；上传由核心按偏移算） | `MediaTask` |
| `/tasks/{id}/complete` | `POST` | 无（下载写完、改好名） | `MediaTask` |
| `/tasks/{id}/failure` | `POST` | `MediaFailure`（设备那边的失败，退避由核心按类别定） | `MediaTask` |
| `/tasks/{id}/items/{item}/failure` | `POST` | `MediaFailure`（上传的一项在设备那边失败，其余照常） | `MediaTask` |
| `/sessions` | `POST` | `PlaybackRequest`（含 `position_ms`、`person`、`device`、`profile`） | `PlaybackSession`（`plan` 已填） |
| `/sessions` | `GET` | 无（query: `person?`） | `{sessions: Vec<PlaybackSession>}` |
| `/sessions/{id}` | `GET` | 无 | `PlaybackSession` |
| `/sessions/{id}/control` | `POST` | `MediaAction` | `PlaybackSession` |
| `/sessions/{id}/progress` | `POST` | `{position_ms, wall_clock}` | `204` |
| `/sessions/{id}/stop` | `POST` | `{position_ms, reason}` | `204` |
| `/queue` | `GET` | 无（query: `person`） | `Queue` |
| `/queue` | `POST` | `QueueOp{revision, op}` | `Queue`；`revision` 不相等回 `409` + 当前 `Queue` |
| `/queue` | `PUT` | 整条 `Queue`（核心推给提供方的 `savePlayQueue` 翻法，带 `position`） | `204` |
| `/tasks/{id}/uploads/{item}` | `POST` | 无体；核心从任务的 `UploadItem` 补体转发 `POST /uploads` | `{upload_offset}` |
| `/tasks/{id}/uploads/{item}` | `PATCH` | 从偏移起的字节，带 `Content-Range`；核心按 `(task,item)` 查 `upload_id` 转发 | `{upload_offset}`；偏移不符 409 带当前偏移 |
| `/tasks/{id}/items/{item}/content` | `GET` | 原始字节，`Range` 原样 | 字节流（206/200） |
| `/sessions/{id}/stream/{文件名}` | `GET` | 原始字节；客户端照计划里的 `stream` 来取 | 字节流（206/200） |
| `/sessions/{id}/items/{item}/stream` | `GET` | 原始字节；`item` 必须在这个人的队列里 | 字节流（206/200）；不在队列里回 `media_gone`。核心按会话的设备档、向这一项自己的来源要它的计划，计划记到过期，不改会话 |
| 上面两条后接 `/{rest}` | `GET` | 原始字节；`rest` 是播放器按上一份列表解析出的相对地址（HLS 的子列表、分片） | 字节流。核心把 `rest` 逐段接到流地址的目录后面，查询串用这次请求的；`.`、`..`、空段回 `protocol` |
| `/sessions/{id}/cast-url` | `POST` | `{target: "airplay"\|"cast"}` | `{url, expires_at}`（绑 LAN 的一次性绝对地址） |
| `/items` | `GET` | 无（query 同提供方的 `GET /items`，`kind` 必填） | `{items, cursor?}`，核心转给这个种类的提供方 |
| `/library/{rest}`、`/catalog/{rest}` | `GET`、`POST` | 同提供方的 `/library/…`、`/catalog/…`（第 9、10 节）；可带 `source` | 提供方的回包原样回；核心按 `library`+`photo`、`catalog`+`video` 选提供方，路径原样转、去掉 `source`，`.`、`..`、空段回 `protocol` |
| `/items/{id}/artwork` | `GET` | 无（query: `kind`，可选 `source`、`type`、`width`） | 图片字节，核心转给提供方的 `GET /items/{id}/artwork`，`type`、`width` 原样带过去 |
| `/commands` | `GET` | 无（query: `device`, `after: seq`） | 长轮询；`{commands: Vec<DeviceCommand>}`，超时 `204` |
| `/commands/{id}/ack` | `POST` | `{seq, ok, failure?}` | `204`（幂等） |
| `/offline` | `PUT` | `{bytes}`：这台设备的离线副本现在一共占多少 | `QuotaView`；这台设备那份记成 `bytes`，别的设备那份不动（规则见 `OfflineUsage`） |
| `/quota` | `GET` | 无 | `QuotaView`：这个人的两本账，上传是各提供方之和、离线是各设备之和 |

`{item}` 是 `UploadItem.item_id`；`(task,item) → upload_id` 的映射只在核心，客户端不记。

命令体是 `DeviceCommand` 与 `MediaAction`，类型定义在上面 `media/view.rs` 一处，这里不重复。

请求体与响应体的字段在 `wonton-protocol` 的 `media/api.rs`（核心接口）和 `media/provider.rs`（提供方契约），这里不重复。

播放计划里的流地址：提供方给的 `stream` 只取路径和查询串，核心按这个提供方的 `media_source` 服务地址去取（主机名不认），请求带同样的凭据与 `X-Wonton-User`；给客户端的计划里 `stream` 换成 `/sessions/{id}/stream/{文件名}`（提供方流地址的最后一段），查询串和 `origin_stream` 不下发（可能带提供方的令牌），取流本身时核心用计划里的。HLS 这类流的子列表、分片写相对地址，播放器按这个地址解析成 `/sessions/{id}/stream/…` 再来取，核心不改写播放列表；只给 `/sessions/{id}/stream` 的话相对地址会落到 `/sessions/{id}/` 下面。

条目的封面：提供方给的 `artwork_url` 只表示有没有图。核心转出去的条目里，有图的换成核心的 `/items/{id}/artwork?kind=…&source=…`（`api::item_artwork_path`），客户端要哪一张、多宽再接 `&type=…&width=…`；核心取图走契约的 `GET /items/{id}/artwork`，不跟提供方给的地址走。

### 选哪个提供方

`media_source` 可以同时有几个提供方（Immich、Navidrome、Jellyfin 可能并存，Immich 和 Jellyfin 都有视频），所有操作走同一条规则：

1. 请求带了来源就用它：条目的 `MediaItemRef.source`（起播、下载、取封面）、上传的 `TaskOptions.source`、列条目和相册浏览的
   `?source=`。这一家不在跑、没声明这次的操作或种类回 `plugin_unavailable`，这个人用不了它回 `permission_denied`；不改去别家。
2. 没带就在“合格的”里挑：在跑、清单声明的 `operations` 含这次的操作、`kinds` 含条目的种类（上传只看 `upload`）、这个人
   用得了。这个人用不了的那家不算合格：别家合格就用别家，不因为它回 `source_required`；能做的都用不了才回
   `permission_denied`。合格的里有主人在能力设置给 `media_source` 定的默认，用默认。
3. 没定默认、合格的只有一家，用它。
4. 合格的有几家又没定默认，回 `source_required`，App 请用户选一家再带 `source` 来；不静默取“第一个”。一家都没有回
   `plugin_unavailable`。
5. 选中的那家再按运行时自报（`GET /_wonton/capabilities`）核对，对不上回 `plugin_unavailable`，不换别家。

- 核心转出去的条目（列条目、浏览、相册、会话里的条目和队列）都由核心填上 `source`，提供方自己填的不算数。
- 上传项和播放会话一建好就记下提供方（上传项连同 `upload_id`），之后的续写、进度、停止都发给它；换提供方只影响新建的。上传点了名，建任务时就按上面的规则核一遍；重试的任务沿用原任务记下的那家。
- 记下的那家每次调用前按这次的操作和种类再核对一遍（同第 5 条），插件升级去掉了能力回 `plugin_unavailable`。
- 会话换曲（`Next`/`Prev`）和取队列里另一项的流，按那一项的来源选：队列是相册、音乐、影视共用的一条。换了家，会话就记到新的那家，和新计划一起写，不会计划是一家、提供方是另一家。
- 队列改成了推给正在播的那家（`PUT /playback/{session}/queue`），只带来源是它或没写来源的条目；没有在播的会话不推，推不过去只记日志。
- 清单的 `provides.media_source.browse`（如 `opensubsonic`）只说这一家的浏览走哪种协议，不参与选提供方。

### 核心对提供方的凭据

核心是使用方，凭据按人各一把：名字是 `wonton-core.media.<人>`，第一次替这个人调某个提供方时现签（执行它的 `credentials.issue`），存在核心数据目录、只有核心读得到。
每个请求带 `Authorization: Bearer <这把>` 和 `X-Wonton-User: <人>`。这个人不在插件的可用范围里就不签，回 `permission_denied`。退出家庭时先 `POST /revoke {name}`，再执行 `credentials.revoke`，最后删掉这把。插件已经卸了的只删核心这把（凭据值只有核心拿着）；没卸只是停了的，成员移除停在这一步，等插件开起来再重试。会话停在 `removed`。

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
| `wonton__backup_now` | `{device?, kinds?}` | `MediaTask` |
| `wonton__media_download` | `{device, item, kind}` | `MediaTask` |
| `wonton__media_play` | `{device?, item, kind, person?}` | `PlaybackSession` |
| `wonton__media_control` | `{session, ctl: ControlOp}` | `PlaybackSession` |
| `wonton__media_queue` | `{op: QueueOp\|read, revision?}` | `Queue` |
| `wonton__media_sessions` | `{person?}` | 这个人的全部设备与会话能力 |
| `wonton__media_task` | `{task, op: pause\|resume\|cancel\|retry}` | `MediaTask` |
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