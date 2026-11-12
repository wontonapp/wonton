//! 文件服务的接口（ADR-0020）：编号和链接、文件信息、错误类别、传输。
//!
//! `/_wonton/files/…` 收发这里的类型，客户端核心把卡片和传输交给壳。核心可能比 App 新：核心多给的字段被忽略；枚举遇到
//! 不认识的取值时落到 `Other` 或 `Unknown`。

mod link;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::chat::RoomRow;

pub use link::{FileId, FileLinks, InvalidFileId, extract_file_links, files_message_body};

/// 所有文件接口都要带的请求头，值是 `1`。隧道身份随 WebView 的每个请求自动带上，跨站带自定义请求头却要先过预检，
/// 插件网页因此碰不到文件接口（ADR-0020 的“鉴权”）。
pub const FILES_HEADER: &str = "X-Wonton-Files";
/// 错误响应里写类别的响应头，值是 [`FileFailure::as_str`]。
pub const FILE_ERROR_HEADER: &str = "X-Wonton-Error";

/// `GET /_wonton/files/{id}` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FileInfo {
    pub id: FileId,
    pub name: String,
    /// MIME 类型；插件资源没有声明时为空。
    pub mime_type: Option<String>,
    /// 当前的字节数；插件资源没有声明时为空。
    pub size: Option<u64>,
    pub source: FileSource,
    pub has_thumbnail: bool,
}

/// 文件从哪来。Bot 分享来的文件内容不可信，卡片要写明来源。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileSource {
    /// 已配对的设备上传的。
    Device,
    /// Bot 经工具结果登记的：`name` 是 Bot，`plugin` 是工具所属的插件。
    Bot { name: String, plugin: String },
    /// 比这个 App 新的核心给的来源。
    #[serde(other)]
    Unknown,
}

/// 消息下面的一张文件卡片。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileCard {
    Loading,
    Ready {
        info: FileInfo,
    },
    /// 看不了，`message` 写明原因。
    Failed {
        reason: FileFailure,
        message: String,
    },
}

/// 卡片上一个按钮做什么。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileCardAction {
    /// 用 `file_stream_url` 的地址放进 App 自己的播放器，边下边播。
    Play,
    /// 发 [`Action::DownloadFile`](crate::Action::DownloadFile)，下完交给系统打开。
    Download,
    /// 文件已经在手机上，把 `path` 交给系统打开。
    Open { path: String },
    /// 这个文件正在下载：等它下完。
    Wait,
}

/// 一张 [`FileCard::Ready`] 上的按钮和下载进展，核心按文件类型、下载记录和手机上的文件给出，壳照着画。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FileCardActions {
    /// 点卡片做什么。
    pub tap: FileCardAction,
    /// “用其他 App 打开”做什么：下载、打开，或者正在下载时等它下完。`tap` 已经是这几种之一时为空。
    pub open_elsewhere: Option<FileCardAction>,
    /// 这个文件最近一次下载；没下载过，或者记录清掉了时为空。
    pub download: Option<Transfer>,
}

/// 文件操作失败的原因。JSON 写法就是 HTTP 错误体的 `kind`、`X-Wonton-Error` 和 Bot 工具错误开头的类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum FileFailure {
    /// 不是经隧道的已配对设备，或者没带 [`FILES_HEADER`]。
    Unauthenticated,
    /// 这台设备的用户不在访问范围里。
    NotShared,
    /// 家里的服务器上路径不合法；经插件拿到文件的 Bot 调工具时，来源插件的授权收回了。
    PermissionDenied,
    /// 没有这个编号。
    FileUnknown,
    /// Bot 登记的编号没分享就过期了。只有工具会返回。
    FileExpired,
    /// 原文件已被移动或删除。
    FileGone,
    /// 文件登记后改过（工具）；要删的已经不是上传的那个文件，编号删了、文件没删（`DELETE`）。
    FileChanged,
    /// 这个类型没有预览或缩略图（`wonton__view_file`、`/thumbnail`）。
    NotViewable,
    /// 家里服务器的磁盘放不下这个上传。
    StorageFull,
    /// 来源插件给不了这个文件：卸了、没在运行、连不上、响应中途断了、读取超过期限，占着名额的读取超过 30 秒没结束，或者不再提供 MCP 服务。说明里写是哪一种。
    PluginUnavailable,
    /// 来源插件返回错误，说明里附上原话。
    UpstreamError,
    /// 插件的资源超过 `resources/read` 的上限：插件要给内网地址。
    ResourceTooLarge,
    /// 核心依赖的系统服务（tusd、Matrix 服务器）没配置或连不上，或者读取、解码的名额满了（忙，不归给来源，稍后再试）。
    ServiceUnavailable,
    /// 家里核心自己的问题：读写不了记录、读不了插件记录、Docker 出错、起不了进程。
    Internal,
    /// 请求本身不合法：请求体读不懂或者没有传完，或者上传没有声明长度。
    InvalidInput,
    /// 客户端核心连不上家里的服务器：隧道没连上、网络断开或请求超时。不会出现在核心的响应里。
    TransportUnavailable,
    /// 客户端核心读不懂核心的响应。不会出现在核心的响应里。
    Protocol,
    /// 手机上要上传的文件读不了：被删了、被改过，或者内容 URI、书签失效了，要重新选择。不会出现在核心的响应里。
    LocalSourceUnavailable,
    /// 系统不让 App 读写手机上的这个文件：选文件时给的读权限丢了，或者下载的位置写不了。不会出现在核心的响应里。
    LocalPermissionDenied,
    /// 手机放不下下载的文件。不会出现在核心的响应里。
    LocalStorageFull,
    /// 手机上读写文件出了别的错，说明里附上系统的原话。不会出现在核心的响应里。
    LocalIo,
    /// 文件拿得到，这台手机的播放器放不了这个格式。不会出现在核心的响应里。
    UnsupportedFormat,
    /// 比这个 App 新的核心给的类别。
    #[serde(other)]
    Other,
}

impl FileFailure {
    /// 类别的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::NotShared => "not_shared",
            Self::PermissionDenied => "permission_denied",
            Self::FileUnknown => "file_unknown",
            Self::FileExpired => "file_expired",
            Self::FileGone => "file_gone",
            Self::FileChanged => "file_changed",
            Self::NotViewable => "not_viewable",
            Self::StorageFull => "storage_full",
            Self::PluginUnavailable => "plugin_unavailable",
            Self::UpstreamError => "upstream_error",
            Self::ResourceTooLarge => "resource_too_large",
            Self::ServiceUnavailable => "service_unavailable",
            Self::Internal => "internal",
            Self::InvalidInput => "invalid_input",
            Self::TransportUnavailable => "transport_unavailable",
            Self::Protocol => "protocol",
            Self::LocalSourceUnavailable => "local_source_unavailable",
            Self::LocalPermissionDenied => "local_permission_denied",
            Self::LocalStorageFull => "local_storage_full",
            Self::LocalIo => "local_io",
            Self::UnsupportedFormat => "unsupported_format",
            Self::Other => "other",
        }
    }

    /// 文件接口用哪个 HTTP 状态码返回这个类别；只有工具或客户端核心才有的类别为空。
    pub fn http_status(self) -> Option<u16> {
        match self {
            Self::Unauthenticated => Some(401),
            Self::NotShared | Self::PermissionDenied => Some(403),
            Self::FileUnknown | Self::NotViewable => Some(404),
            Self::FileGone => Some(410),
            Self::FileChanged => Some(409),
            Self::StorageFull => Some(507),
            Self::PluginUnavailable | Self::ServiceUnavailable => Some(503),
            Self::UpstreamError | Self::ResourceTooLarge => Some(502),
            Self::Internal => Some(500),
            Self::InvalidInput => Some(400),
            Self::FileExpired
            | Self::TransportUnavailable
            | Self::Protocol
            | Self::LocalSourceUnavailable
            | Self::LocalPermissionDenied
            | Self::LocalStorageFull
            | Self::LocalIo
            | Self::UnsupportedFormat
            | Self::Other => None,
        }
    }
}

/// 文件接口的错误响应体：`{"error": {"kind": …, "message": …}}`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileErrorBody {
    pub error: FileError,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileError {
    pub kind: FileFailure,
    pub message: String,
}

/// `POST /_wonton/files/{id}/grants` 的请求体：把这个房间的 Bot 加进访问范围。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileGrantRequest {
    pub room: String,
}

/// 要上传的文件从哪里读。App 重启后接着传时再打开一次。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UploadSource {
    /// 客户端核心自己能打开的本机绝对路径。iOS 上 App 容器的路径在 App 更新后会变，接着传就找不到了，要用书签。
    Path { path: String },
    /// Android 的内容 URI、iOS 的书签：客户端核心不解析，原样交给壳的 `FileOpener`。
    Opaque { handle: String },
}

/// 一次上传或下载，存在本机。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Transfer {
    /// 本机给这次传输的标识，交给 [`actions`](Self::actions) 对应的操作。不用文件编号：上传在核心发编号之前就要显示进度，
    /// 创建时的 `storage_full` 也要落在这条传输上。
    pub id: String,
    /// 文件的编号；上传还没创建时为空。
    pub file: Option<FileId>,
    pub name: String,
    pub direction: TransferDirection,
    /// 上传的目标房间，链接发到这里；下载和不发进房间的上传为空。
    pub room: Option<String>,
    pub transferred_bytes: u64,
    /// 总字节数；下载还没拿到响应头时为空，核心没给 `Content-Length` 时一直为空。
    pub total_bytes: Option<u64>,
    pub state: TransferState,
    /// [`TransferState::Paused`] 时家里的服务出了什么问题；为空就是在等连接恢复。
    pub paused_because: Option<String>,
    /// 下载放在 App 目录里的绝对路径，[`TransferState::Done`] 之后壳交给系统打开或分享；上传时、下载还没拿到文件名时为空。
    /// 不要存下来：iOS 上 App 更新后容器的路径会变。
    pub destination: Option<String>,
    /// 这条传输上放哪些按钮，按这个顺序。
    pub actions: Vec<TransferAction>,
}

/// 传输上的一个按钮。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum TransferAction {
    /// 发 [`Action::CancelTransfer`](crate::Action::CancelTransfer)。
    Cancel,
    /// 发 `delete_local_file` 为假的 [`Action::DismissTransfer`](crate::Action::DismissTransfer)。
    Dismiss,
    /// 先问主人只清记录，还是连手机上的文件一起删，再发 [`Action::DismissTransfer`](crate::Action::DismissTransfer)。
    DismissDownloaded,
    /// 发 [`Action::ResendFileLink`](crate::Action::ResendFileLink)。
    ResendLink,
    /// 字节已经传完，只差发链接时的“不发了”：发 [`Action::CancelTransfer`](crate::Action::CancelTransfer)。文件留在服务器上，
    /// 不是删了。
    StopSending,
    /// 把 [`Transfer::destination`] 交给系统打开。
    Open,
    /// 把 [`Transfer::destination`] 交给系统的分享或保存。
    Share,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransferState {
    Running,
    /// App 进了后台、断了网，或者家里的服务不可用（[`Transfer::paused_because`]）。回到前台、连上之后接着传。
    Paused,
    Done,
    /// 文件已经传完，在服务器上；授权或发进房间这一步没成。不自动再试：用
    /// [`Action::ResendFileLink`](crate::Action::ResendFileLink) 只重发链接，不用重传文件。
    LinkNotSent {
        reason: FileFailure,
        message: String,
    },
    Failed {
        reason: FileFailure,
        message: String,
    },
}

/// 传输列表，存在本机：App 重启后还在，接着传。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TransfersViewModel {
    /// 进行中的，和要主人处理的（失败、链接没发出）。
    pub active: Vec<Transfer>,
    /// 下载完的：文件在 App 自己的目录里，主人清理之前一直留着。
    pub downloaded: Vec<Transfer>,
    /// 上传完的。不占主人的地方，只留最近的一些。
    pub uploaded: Vec<Transfer>,
    /// 最近一次不落在某条传输上的文件操作为什么失败，壳显示后用
    /// [`Action::DismissFileProblem`](crate::Action::DismissFileProblem) 清掉。
    pub problem: Option<FileProblem>,
}

/// 一个房间的传输条要列什么（`ClientCore::room_transfers`）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoomTransfers {
    /// 目标是这个房间的，和没有目标房间的（下载、不发进房间的上传）。
    pub here: Vec<Transfer>,
    /// 发到别的房间、出了问题的上传：链接传完才发，时间线上没有痕迹，主人不在那个房间就不知道。
    pub elsewhere: Vec<TransferElsewhere>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TransferElsewhere {
    pub transfer: Transfer,
    /// 目标房间的名字。房间列表里没有它时为空：换了家里的服务器，或者退出了那个房间。
    pub room_name: Option<String>,
}

impl TransfersViewModel {
    /// 这个房间的传输条列什么。进行中的上传只在目标房间显示：在别处点“重发链接”会以为发到眼前这里，所以别处的都写明目标房间。
    pub fn for_room(&self, room_id: &str, rooms: &[RoomRow]) -> RoomTransfers {
        let mut listed = RoomTransfers::default();
        for transfer in &self.active {
            match &transfer.room {
                Some(room) if room != room_id => {
                    if matches!(
                        transfer.state,
                        TransferState::Failed { .. } | TransferState::LinkNotSent { .. }
                    ) {
                        listed.elsewhere.push(TransferElsewhere {
                            transfer: transfer.clone(),
                            room_name: rooms
                                .iter()
                                .find(|row| row.room_id == *room)
                                .map(|row| row.name.clone()),
                        });
                    }
                }
                _ => listed.here.push(transfer.clone()),
            }
        }
        listed
    }

    /// 三个列表里的所有传输。
    pub fn all(&self) -> impl Iterator<Item = &Transfer> {
        self.active
            .iter()
            .chain(&self.downloaded)
            .chain(&self.uploaded)
    }
}

/// 不落在某条传输上的文件操作。`Play` 只出现在 `file_playback_problem` 的结果里，不进 [`TransfersViewModel::problem`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum FileOperation {
    Delete,
    SendToDevices,
    GrantToRoom,
    Play,
}

/// 一次文件操作失败的原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FileProblem {
    pub operation: FileOperation,
    pub file: FileId,
    pub reason: FileFailure,
    /// 给人看的说明，包含失败的操作、文件和原因。
    pub message: String,
}
