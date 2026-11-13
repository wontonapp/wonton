//! 从系统分享面板分享进云吞（ADR-0097）：壳把收到的东西放进收件目录，核心认领后给出分享页的 ViewModel，主人选好聊天点发送，
//! 由附件区（ADR-0094）的同一段代码发出去。
//!
//! 收件目录的格式是壳（Android 的 Activity、iOS 的分享扩展）和核心之间的契约，只在这里定：
//!
//! ```text
//! <数据目录>/share-inbox/<令牌>/
//!     manifest.json   ShareInManifest
//!     <文件>…         manifest 里每项的 file
//!     .ready          最后写：有它才表示这次分享写完整了
//! ```
//!
//! iOS 的扩展不跑核心（ADR-0097 第 3 节），写完后用 `wonton://share/<令牌>` 打开 App；Android 写完直接调
//! `ClientCore::claim_share`。

use serde::{Deserialize, Serialize};

use crate::{AttachmentKind, ImageUpload, RoomRow};

/// 数据目录下的收件目录名。
pub const SHARE_INBOX_DIR: &str = "share-inbox";
/// 每次分享的清单文件名。
pub const SHARE_MANIFEST_FILE: &str = "manifest.json";
/// 写完整的标记文件名，壳在清单和文件都写好以后最后写它。
pub const SHARE_READY_FILE: &str = ".ready";
/// 一次分享最多几项，和附件区的上限（`ComposerViewModel::max_attachments`）一样。
pub const SHARE_MAX_ITEMS: u32 = 10;
/// 清单最大字节数：只有文字和文件名，再大就不是壳写的。
const MANIFEST_MAX_BYTES: usize = 1024 * 1024;

/// 分享页。没有待发的分享时 `pending` 为空，壳收起分享页。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ShareInViewModel {
    pub pending: Option<PendingShare>,
}

/// 分享进来、还没发出去的一次分享。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PendingShare {
    /// 收件目录的令牌，也是这次分享的标识。
    pub id: String,
    /// 照片和文件，按分享进来的顺序，也是发出去的顺序。
    pub items: Vec<ShareInItem>,
    /// 分享进来的文字（链接也在这里），壳把它放进输入框，主人可以改。
    pub text: String,
    /// 能发去哪里。
    pub destinations: ShareInDestinations,
    /// 超过 [`SHARE_MAX_ITEMS`] 被丢掉的项数，壳提示一句。
    pub dropped: u32,
}

/// 分享进来的一项。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ShareInItem {
    /// 这一项的标识，交给 `ClientCore::discard_share` 和 [`ShareInImage::item_id`]。
    pub id: String,
    /// 照片按 `m.image` 发，宽高为空；文件经文件服务发成链接。
    pub kind: AttachmentKind,
    pub name: String,
    pub mime_type: Option<String>,
    /// 字节数。
    pub size: u64,
    /// 本机的绝对路径：照片由壳读出来压缩、去掉 EXIF（和从相册选的一样）再交回，缩略图也由壳从这里画。
    /// 不要存下来，发出去或丢掉以后文件会删掉。
    pub path: String,
}

/// 分享页的去向。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ShareInDestinations {
    /// 这台手机还没配对家里的服务器：分享页照常显示条目，引导主人去配对，配好后回来接着发。
    NotPaired,
    /// 聊天还在登录或同步，房间列表还没到。
    Loading,
    /// 聊天登录失败、不会自动再试：`message` 是默认收起的详情，按 App 的语言。
    Unavailable { message: String },
    /// 能发的聊天，最近有活动的在前；为空时主人还没有能发的聊天。`suggested_room` 是默认选中的房间：上次分享发去的
    /// 那个，它不在了就是和默认 Agent 的私聊，都没有时为空，由主人自己挑。
    Ready {
        rooms: Vec<RoomRow>,
        suggested_room: Option<String>,
    },
}

/// 发送时壳交回的一张照片：从 [`ShareInItem::path`] 读出来、按 ADR-0005 的规则压缩和去掉 EXIF 以后的结果。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ShareInImage {
    pub item_id: String,
    pub upload: ImageUpload,
}

/// `manifest.json`：壳写，核心读。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareInManifest {
    /// 结构版本，现在是 [`Self::VERSION`]。
    pub v: u32,
    /// 分享进来的文字和链接，没有时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub items: Vec<ShareInManifestItem>,
}

/// 清单里的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareInManifestItem {
    /// 收件目录里的文件名，不带路径。
    pub file: String,
    /// 给人看的文件名；发送方没给时壳自己起一个。
    pub name: String,
    /// 发送方给的 MIME 类型，没有时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

/// 清单读不了的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareInManifestError {
    /// 不是这个结构的 JSON，或者太大。
    Malformed(String),
    /// 更新的壳写的版本。
    UnknownVersion(u32),
    /// 文件名带路径、以点开头或者为空。
    BadFileName(String),
    /// 文字和条目都没有。
    Empty,
}

impl std::fmt::Display for ShareInManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "malformed manifest: {reason}"),
            Self::UnknownVersion(version) => write!(f, "unknown manifest version {version}"),
            Self::BadFileName(name) => write!(f, "bad file name {name:?}"),
            Self::Empty => f.write_str("nothing was shared"),
        }
    }
}

impl ShareInManifest {
    pub const VERSION: u32 = 1;

    /// 读清单并校验。条目多于 [`SHARE_MAX_ITEMS`] 不算错，由认领的一方截断。
    pub fn parse(bytes: &[u8]) -> Result<Self, ShareInManifestError> {
        if bytes.len() > MANIFEST_MAX_BYTES {
            return Err(ShareInManifestError::Malformed(format!(
                "{} bytes",
                bytes.len()
            )));
        }
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| ShareInManifestError::Malformed(error.to_string()))?;
        if manifest.v != Self::VERSION {
            return Err(ShareInManifestError::UnknownVersion(manifest.v));
        }
        if let Some(item) = manifest
            .items
            .iter()
            .find(|item| !is_plain_file_name(&item.file))
        {
            return Err(ShareInManifestError::BadFileName(item.file.clone()));
        }
        let has_text = manifest
            .text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty());
        if manifest.items.is_empty() && !has_text {
            return Err(ShareInManifestError::Empty);
        }
        Ok(manifest)
    }
}

impl ShareInManifestItem {
    /// 照片还是文件：只看 MIME 类型。两端的压缩都认得的几种是照片，其余（包括 SVG、没有类型的）当文件发。
    pub fn kind(&self) -> AttachmentKind {
        let mime = self.mime.as_deref().map(str::to_ascii_lowercase);
        let image = matches!(
            mime.as_deref(),
            Some(
                "image/jpeg"
                    | "image/png"
                    | "image/gif"
                    | "image/webp"
                    | "image/heic"
                    | "image/heif"
            )
        );
        if image {
            AttachmentKind::Image {
                width: None,
                height: None,
            }
        } else {
            AttachmentKind::File
        }
    }
}

/// 收件目录的令牌：8 到 64 个字母、数字或连字符，直接当目录名用。两端用各自的 UUID。
pub fn is_share_token(token: &str) -> bool {
    (8..=64).contains(&token.len())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && name.len() <= 255
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(value: &serde_json::Value) -> Result<ShareInManifest, ShareInManifestError> {
        ShareInManifest::parse(value.to_string().as_bytes())
    }

    /// 契约：两端写的清单（ADR-0097 第 4 节）。Swift 的 `JSONEncoder` 和 Kotlin 的 `JSONObject` 按这个样子写。
    #[test]
    fn a_manifest_written_by_the_shells_is_read() {
        let manifest = parse(&json!({
            "v": 1,
            "text": "哪张好看",
            "items": [
                {"file": "0.jpg", "name": "IMG_0001.JPG", "mime": "image/jpeg"},
                {"file": "1", "name": "账单.pdf", "mime": "application/pdf"},
                {"file": "2", "name": "notes"}
            ]
        }))
        .unwrap();
        assert_eq!(manifest.text.as_deref(), Some("哪张好看"));
        let kinds: Vec<_> = manifest
            .items
            .iter()
            .map(ShareInManifestItem::kind)
            .collect();
        assert_eq!(
            kinds,
            [
                AttachmentKind::Image {
                    width: None,
                    height: None
                },
                AttachmentKind::File,
                AttachmentKind::File,
            ]
        );
    }

    #[test]
    fn text_alone_is_a_share() {
        let manifest = parse(&json!({"v": 1, "text": "https://example.org/a"})).unwrap();
        assert!(manifest.items.is_empty());
    }

    #[test]
    fn a_manifest_without_text_or_items_is_rejected() {
        assert_eq!(
            parse(&json!({"v": 1, "text": "  ", "items": []})),
            Err(ShareInManifestError::Empty)
        );
    }

    #[test]
    fn an_unknown_version_is_rejected() {
        assert_eq!(
            parse(&json!({"v": 2, "text": "a"})),
            Err(ShareInManifestError::UnknownVersion(2))
        );
    }

    #[test]
    fn file_names_cannot_leave_the_inbox() {
        for bad in ["../servers.json", "a/b", ".ready", ""] {
            let result = parse(&json!({"v": 1, "items": [{"file": bad, "name": "x"}]}));
            assert_eq!(
                result,
                Err(ShareInManifestError::BadFileName(bad.into())),
                "{bad}"
            );
        }
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(matches!(
            parse(&json!({"v": 1, "text": "a", "target": "!room"})),
            Err(ShareInManifestError::Malformed(_))
        ));
    }

    #[test]
    fn tokens_are_directory_safe() {
        assert!(is_share_token("6F9619FF-8B86-D011-B42D-00C04FC964FF"));
        assert!(is_share_token("3f0c9a1e5b7d42c6"));
        for bad in ["short", "../../etc", "a b c d e f g h", &"a".repeat(65)] {
            assert!(!is_share_token(bad), "{bad}");
        }
    }
}
