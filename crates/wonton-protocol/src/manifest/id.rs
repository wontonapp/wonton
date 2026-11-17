use std::fmt;

use serde::{Deserialize, Serialize};
/// 插件 ID 的最大长度。网关用它派生子域名，必须放得进一个 DNS 标签。
pub const MAX_ID_LEN: usize = 63;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error(
        "插件 ID `{0}` 不合法：须为反向域名形式（小写字母、数字、连字符，至少两段，总长不超过 63）"
    )]
    InvalidId(String),
    #[error("版本号 `{0}` 不是语义化版本（如 1.2.3）")]
    InvalidVersion(String),
    #[error("`{field}` 至少要写一种语言，写了的语言不能是空的")]
    MissingLocale { field: &'static str },
    #[error("权限 `{0}` 不在规范的权限表里")]
    UnknownPermission(String),
    #[error("插件按规范 {spec} 写成，这个核心只支持到 {supported}：先升级核心")]
    NewerSpec { spec: String, supported: String },
    #[error("{0}")]
    Invalid(String),
}

/// 插件 ID：反向域名形式，如 `app.immich.photos`。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(String);

impl PluginId {
    pub fn parse(value: &str) -> Result<Self, ManifestError> {
        let segments: Vec<&str> = value.split('.').collect();
        let valid = value.len() <= MAX_ID_LEN
            && segments.len() >= 2
            && segments.iter().all(|segment| is_label(segment));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(ManifestError::InvalidId(value.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 网关为插件网页分配的子域名标签：把 `.` 换成 `-`。
    pub fn slug(&self) -> String {
        self.0.replace('.', "-")
    }
}

/// 一段由小写字母和数字组成、可用单个连字符分隔的标签。
pub fn is_label(segment: &str) -> bool {
    !segment.is_empty()
        && segment.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

impl TryFrom<String> for PluginId {
    type Error = ManifestError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<PluginId> for String {
    fn from(id: PluginId) -> Self {
        id.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
