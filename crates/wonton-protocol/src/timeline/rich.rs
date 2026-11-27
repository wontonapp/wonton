//! 排好版的富文本块和文字里的链接。

use serde::{Deserialize, Serialize};

/// 排好版的一块文字。没有嵌套：列表和引用的层次摊平成每一块的 `depth` 和 `quoted`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RichBlock {
    pub kind: RichBlockKind,
    /// 缩进几层。
    pub depth: u32,
    /// 在引用里。
    pub quoted: bool,
    pub spans: Vec<RichSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RichBlockKind {
    Paragraph,
    Heading,
    /// `marker` 是项目符号、序号或任务框，壳原样画在文字前面。
    ListItem {
        marker: String,
    },
    /// 代码块：等宽字体，不折行，横向滚动。
    Code,
    /// 表格，第一行是表头，每行格数相同。不折行，横向滚动。
    Table {
        rows: Vec<Vec<String>>,
    },
    /// 分隔线，`spans` 为空。
    Rule,
}

/// 一段样式相同的文字。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RichSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<RichLink>,
}

/// 文字里的一个链接，以及核心判断的类别：壳按类别打开，不自己看地址（ADR-0065）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RichLink {
    /// 打开或交给系统的地址。不带协议的网址已经补成 http，纯文本邮箱补成 `mailto:`，号码补成 `tel:`。
    pub url: String,
    pub kind: RichLinkKind,
}

/// 链接的类别，决定点开以后去哪里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RichLinkKind {
    /// 别处的网页：iOS 用 `SFSafariViewController`、Android 用 Custom Tabs 打开。
    External,
    /// 这台服务器上插件的网页：打开那个插件的页面，经隧道。
    Plugin { plugin_id: String },
    /// matrix.to 的链接：在 App 里打开对应的用户或房间。
    Matrix { target: MatrixTarget },
    /// 邮件：交给系统。
    Email,
    /// 电话：交给系统。
    Phone,
    /// 云吞自己的 `wonton://` 等其他协议地址：`wonton` 由壳交给 App 的链接表就地打开，别的交给系统。
    Other,
}

/// matrix.to 链接指向的东西。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MatrixTarget {
    /// 一个用户。
    User { user_id: String },
    /// 一个房间：房间 ID，或房间别名（`#alias:server`）。`event_id` 是链接指向的事件（事件永久链接），只有房间时为 `None`。
    Room {
        room_id: String,
        event_id: Option<String>,
    },
}
