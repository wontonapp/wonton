//! Bot 的云朵形象（ADR-0126 的“Agent 的云朵形象”“头像分层和素材库”）。
//!
//! 头像是身体、表情、头饰三层，由核心按素材合成 [`CLOUD_AVATAR_EDGE`] 见方的 PNG，垫上从身体色算出的极浅圆底
//! （不存），以 Bot 的身份写进 Matrix 资料：静态头像（`avatar_url`）用人挑的默认表情，状态头像进
//! [`AVATARS_FIELD`](crate::AVATARS_FIELD)。人的选择存在 Bot 资料的自定义字段 [`CLOUD_FIELD`]，格式是 [`BotCloud`]。
//! 壳只挑：颜色给 RGB，表情和头饰给素材 ID；合成和上传都不在壳里。
//!
//! 素材 ID 带命名空间：`builtin:<种类>/<键>` 是编进核心的内置素材，`user:<uuid>` 是这个人素材库里的，
//! 插件和 Agent 加的也是 `user:`。

use serde::{Deserialize, Serialize};

use crate::AvatarImage;

/// Bot 资料里云朵那个字段的名字（MSC4133，和 [`AVATARS_FIELD`](crate::AVATARS_FIELD) 并列）。
/// 选“自己的图”时核心清掉它；有它时头像来源是云朵。
pub const CLOUD_FIELD: &str = "app.wonton.cloud";

/// `GET <base>/_wonton/api/cloud-assets?plugin=<插件 ID>`：内置的、这个人的、这个插件给的素材清单（[`CloudAssets`]）；
/// `POST` 由主人往自己的库加素材（Owner 令牌）。单项见 [`cloud_asset_path`]。
pub const CLOUD_ASSETS_PATH: &str = "/_wonton/api/cloud-assets";

/// `GET` 取单项带原图，`DELETE` 只删人自己库里的。ID 里的 `:` 和 `/` 编码进一段路径。
///
/// 编码用 [`NON_ALPHANUMERIC`](percent_encoding::NON_ALPHANUMERIC)，`-`、`_`、`.` 也会被编码；
/// 服务端要先按 `/` 切段、再解这一个段，不能把整条路径先解码（`builtin:face/smile` 里的 `/` 会变成段分隔）。
pub fn cloud_asset_path(id: &str) -> String {
    let id = percent_encoding::utf8_percent_encode(id, percent_encoding::NON_ALPHANUMERIC);
    format!("{CLOUD_ASSETS_PATH}/{id}")
}

/// 总线工具：插件和 Agent 往调用方归属人的素材库加一个素材，返回 `user:<uuid>`。
pub const ADD_AVATAR_ASSET_TOOL: &str = "wonton__add_avatar_asset";

/// 合成出的头像边长（像素）。
pub const CLOUD_AVATAR_EDGE: u32 = 512;
/// 素材的边长：透明底 PNG，[`CLOUD_ASSET_EDGE`] 见方，误差不超过 [`CLOUD_ASSET_EDGE_TOLERANCE`]。
pub const CLOUD_ASSET_EDGE: u32 = 1024;
pub const CLOUD_ASSET_EDGE_TOLERANCE: u32 = 1;
/// 每个素材文件的大小上限。
pub const MAX_CLOUD_ASSET_BYTES: u64 = 2 * 1024 * 1024;
/// 素材名去掉首尾空白后最多几个字符（Unicode 标量）。
pub const MAX_CLOUD_ASSET_NAME_CHARS: usize = 24;
/// 每个 Bot 每小时最多导入几次、每次最多几个文件（主体加泪滴、口水这类附件），超了返回 `rate_limited`。
pub const MAX_CLOUD_ASSET_IMPORTS_PER_HOUR: u32 = 12;
pub const MAX_CLOUD_ASSET_FILES_PER_IMPORT: u32 = 3;
/// 每人素材库里最多几个素材，到了先删再加。
pub const MAX_USER_CLOUD_ASSETS: u32 = 200;

/// 一个 Bot 的云朵：身体色、默认表情、头饰。资料字段 [`CLOUD_FIELD`] 存的就是它。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotCloud {
    /// `#RRGGBB`，任意颜色值；预设只是 [`CloudAssetKind::BodyPreset`] 的几项。
    pub body: String,
    /// 默认表情的素材 ID，静态头像用它；状态头像按状态换表情，颜色和头饰不变。
    pub face: String,
    /// 头饰的素材 ID；不戴时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headdress: Option<String>,
}

impl BotCloud {
    /// 三项各自的格式：颜色是 `#RRGGBB`，表情和头饰是对应种类的素材 ID。素材存不存在由核心另查。
    pub fn check(&self) -> Result<(), CloudProblem> {
        parse_cloud_color(&self.body).ok_or(CloudProblem::Color)?;
        check_part_id(&self.face, CloudLayer::Face)?;
        match &self.headdress {
            Some(id) => check_part_id(id, CloudLayer::Headdress),
            None => Ok(()),
        }
    }
}
/// 只改云朵的一项（取色器、点一个表情或头饰）。
///
/// 三项的载荷都是标量或没有，用结构体变体（不是一个元组）让 serde 的内部标签平铺、Swift/Kotlin 的绑定也带字段名。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "part", rename_all = "snake_case")]
pub enum CloudPart {
    /// `#RRGGBB`。
    Body {
        color: String,
    },
    Face {
        id: String,
    },
    /// 为空是不戴。
    Headdress {
        id: Option<String>,
    },
}

/// 云朵的一项为什么不行。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum CloudProblem {
    /// 不是 `#RRGGBB`。
    Color,
    /// 不是 `builtin:<种类>/<键>` 或 `user:<uuid>`。
    AssetId,
    /// 内置素材的种类不对，如把头饰当表情。
    WrongKind,
    /// 素材库里没有这个 ID。只有核心查得出来。
    NotFound,
    #[serde(other)]
    Other,
}

/// 读 `#RRGGBB`（大小写都收）。
pub fn parse_cloud_color(value: &str) -> Option<crate::Rgb> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some(crate::Rgb {
        red: channel(0)?,
        green: channel(2)?,
        blue: channel(4)?,
    })
}

/// 写成 `#RRGGBB`（大写），存进 [`BotCloud::body`] 的就是这个形式。
pub fn cloud_color(color: crate::Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", color.red, color.green, color.blue)
}

/// 素材 ID 拆开后的样子。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudAssetRef<'a> {
    /// 编进核心的：`builtin:face/smile`。
    Builtin { layer: CloudLayer, key: &'a str },
    /// 这个人素材库里的：`user:<uuid>`。
    User { id: uuid::Uuid },
}

/// 内置素材 ID 里的种类段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudLayer {
    Body,
    Face,
    Headdress,
}

impl CloudLayer {
    fn segment(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Face => "face",
            Self::Headdress => "headdress",
        }
    }
}

/// 拆一个素材 ID；格式不对时为空。键只收小写字母、数字和 `-`。
pub fn parse_cloud_asset_id(id: &str) -> Option<CloudAssetRef<'_>> {
    if let Some(rest) = id.strip_prefix("builtin:") {
        let (segment, key) = rest.split_once('/')?;
        let layer = [CloudLayer::Body, CloudLayer::Face, CloudLayer::Headdress]
            .into_iter()
            .find(|layer| layer.segment() == segment)?;
        let valid_key = !key.is_empty()
            && key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        return valid_key.then_some(CloudAssetRef::Builtin { layer, key });
    }
    let uuid = id.strip_prefix("user:")?;
    // 只收带连字符的小写形式，同一个素材只有一种写法。
    let parsed = uuid::Uuid::parse_str(uuid).ok()?;
    (parsed.hyphenated().to_string() == uuid).then_some(CloudAssetRef::User { id: parsed })
}

fn check_part_id(id: &str, layer: CloudLayer) -> Result<(), CloudProblem> {
    match parse_cloud_asset_id(id).ok_or(CloudProblem::AssetId)? {
        CloudAssetRef::Builtin { layer: found, .. } if found != layer => {
            Err(CloudProblem::WrongKind)
        }
        _ => Ok(()),
    }
}

/// 素材的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum CloudAssetKind {
    /// 预设的身体色，点它就是 [`CloudPart::Body`]。
    BodyPreset,
    Face,
    Headdress,
    /// 比这个 App 新的核心给的种类：壳不列出来。
    #[serde(other)]
    Other,
}

/// 素材从哪来，挑选界面按它分组。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CloudAssetAuthor {
    /// 编进核心的。
    Builtin,
    /// 插件或它的 Agent 经 [`ADD_AVATAR_ASSET_TOOL`] 加的；插件卸掉后素材还在。
    Plugin { plugin_id: String },
    /// 人自己从相册加的。
    User,
    #[serde(other)]
    Other,
}

/// [`CLOUD_ASSETS_PATH`] 清单里的一项。原图用 [`cloud_asset_path`] 取。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudAssetInfo {
    pub id: String,
    pub kind: CloudAssetKind,
    /// 导入方给的名字；内置素材是核心按请求的语言给的。
    pub name: String,
    pub author: CloudAssetAuthor,
    /// 只对 [`CloudAssetKind::BodyPreset`]：预设的 `#RRGGBB`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// [`CLOUD_ASSETS_PATH`] 的响应。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudAssets {
    pub assets: Vec<CloudAssetInfo>,
}

/// 挑选界面的一项素材。内置的和人的同列，壳按 `author` 分组。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CloudAsset {
    pub id: String,
    pub kind: CloudAssetKind,
    pub name: String,
    pub author: CloudAssetAuthor,
    /// 只对 [`CloudAssetKind::BodyPreset`]：预设的 `#RRGGBB`。
    pub color: Option<String>,
    /// 这一项的预览图，壳用 `load_media` 取字节。
    pub preview: AvatarImage,
}

/// 素材名不行的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum CloudAssetNameProblem {
    Empty,
    /// 超过 [`MAX_CLOUD_ASSET_NAME_CHARS`]。
    TooLong,
    /// 有换行或控制字符。
    ControlCharacters,
    /// 和同一种类的内置素材重名。只有核心查得出来；库里同名的两个人素材是两个素材，不算重名。
    Reserved,
    #[serde(other)]
    Other,
}

/// 名字本身的规则：`name` 已经去掉首尾空白。和内置素材重名（[`CloudAssetNameProblem::Reserved`]）由核心另查。
pub fn check_cloud_asset_name(name: &str) -> Result<(), CloudAssetNameProblem> {
    if name.is_empty() {
        return Err(CloudAssetNameProblem::Empty);
    }
    if name.chars().count() > MAX_CLOUD_ASSET_NAME_CHARS {
        return Err(CloudAssetNameProblem::TooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(CloudAssetNameProblem::ControlCharacters);
    }
    Ok(())
}

#[cfg(test)]
#[path = "cloud_tests.rs"]
mod tests;
