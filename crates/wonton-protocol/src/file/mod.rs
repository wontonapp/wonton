//! 文件服务的接口（ADR-0020）：编号和链接、文件信息、错误类别、传输。
//!
//! `/_wonton/files/…` 收发这里的类型，客户端核心把卡片和传输交给壳。核心可能比 App 新：核心多给的字段被忽略；枚举遇到
//! 不认识的取值时落到 `Other` 或 `Unknown`。

mod link;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

pub use link::{FileId, FileLinks, InvalidFileId, extract_file_links};

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

/// 文件操作失败的原因。JSON 写法就是 HTTP 错误体的 `kind`、`X-Wonton-Error` 和 Bot 工具错误开头的类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum FileFailure {
    /// 不是经隧道的已配对设备，或者没带 [`FILES_HEADER`]。
    Unauthenticated,
    /// 这台设备的用户不在访问范围里。
    NotShared,
    /// 路径不合法；Bot 调工具时，来源插件的授权收回了。
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
    /// 磁盘放不下这个上传。
    StorageFull,
    /// 来源插件给不了这个文件：卸了、没在运行、连不上，或者不再提供 MCP 服务。说明里写是哪一种。
    PluginUnavailable,
    /// 来源插件返回错误，说明里附上原话。
    UpstreamError,
    /// 插件的资源超过 `resources/read` 的上限：插件要给内网地址。
    ResourceTooLarge,
    /// 核心依赖的系统服务（tusd、Matrix 服务器）没配置或连不上。
    ServiceUnavailable,
    /// 核心自己的问题：读写不了记录、读不了插件记录、Docker 出错、起不了进程。
    Internal,
    /// 请求本身不合法：请求体读不懂，或者上传没有声明长度。
    InvalidInput,
    /// 客户端核心连不上家里的服务器：隧道没连上、网络断开或请求超时。不会出现在核心的响应里。
    TransportUnavailable,
    /// 客户端核心读不懂核心的响应。不会出现在核心的响应里。
    Protocol,
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
            Self::FileExpired | Self::TransportUnavailable | Self::Protocol | Self::Other => None,
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

/// 一次上传或下载，存在本机。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Transfer {
    /// 本机给这次传输的标识，交给 [`Action::CancelTransfer`](crate::Action::CancelTransfer)。不用文件编号：上传在核心发
    /// 编号之前就要显示进度，创建时的 `storage_full` 也要落在这条传输上；同一个文件也可以同时下载到两个地方。
    pub id: String,
    /// 文件的编号；上传还没创建时为空。
    pub file: Option<FileId>,
    pub name: String,
    pub direction: TransferDirection,
    pub transferred_bytes: u64,
    /// 总字节数；下载还没拿到响应头时为空，核心没给 `Content-Length` 时一直为空。
    pub total_bytes: Option<u64>,
    pub state: TransferState,
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
    /// App 进了后台或者断了网，回到前台、连上之后接着传。
    Paused,
    Done,
    Failed {
        reason: FileFailure,
        message: String,
    },
}
