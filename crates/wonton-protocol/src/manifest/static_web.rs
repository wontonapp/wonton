use serde::{Deserialize, Serialize};

/// `x-wonton.static_web`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticWebSpec {
    /// 包里放静态文件的目录。
    #[serde(default = "default_root")]
    pub root: String,
    /// 找不到的路径回这个文件（单页应用）；没写时回 404。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// 页面能连的外部来源：`https://<主机名>` 或 `wss://<主机名>`，主机名可以以 `*.` 开头。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connect: Vec<String>,
}

impl Default for StaticWebSpec {
    fn default() -> Self {
        Self {
            root: default_root(),
            fallback: None,
            connect: Vec::new(),
        }
    }
}

fn default_root() -> String {
    "web".to_owned()
}

/// `x-wonton.link`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkSpec {
    pub url: String,
}
