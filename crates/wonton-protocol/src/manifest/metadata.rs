use serde::{Deserialize, Serialize};

/// 插件包的作者。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}
