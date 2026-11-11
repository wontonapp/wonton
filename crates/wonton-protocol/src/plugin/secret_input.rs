//! 主人填写的插件密钥（ADR-0064）：安装、升级时在原生表单里填的项，详情页里的填写情况，以及修改单项的接口。
//!
//! 值只在安装、升级和确认卡片的请求里上行；任何响应都不带值，详情只给填没填和最后修改时间。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::SecretString;

/// 修改一项密钥的接口（`PUT`）：填新值，或者把可选项清空。只有主人能调，响应是更新后的
/// [`PluginDetail`](super::PluginDetail)。
pub fn secret_input_path(plugin_id: &str, name: &str) -> String {
    format!("/_wonton/api/plugins/{plugin_id}/secrets/{name}")
}

/// 安装或升级前主人要填的一项密钥（清单 `x-wonton.secrets` 的 `input`）。标题和说明由核心按请求的语言选好。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SecretInput {
    /// 密钥名（清单 `x-wonton.secrets` 的键），也是请求里值的键。
    pub name: String,
    /// 输入框的标题，如“Brave Search API key”。
    pub title: String,
    /// 一句补充，如免费额度、要开哪些权限；作者没写时为空。
    #[serde(default)]
    pub description: Option<String>,
    /// 去哪里拿这个密钥，`https://` 链接，App 显示成链接；作者没写时为空。
    #[serde(default)]
    pub url: Option<String>,
    /// 必填项：缺了装不上、升不了级。
    pub required: bool,
}

/// 详情页里一项密钥的填写情况。没有值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SecretInputState {
    /// 密钥名（清单 `x-wonton.secrets` 的键），修改接口的 `{name}`。
    pub name: String,
    /// 给主人看的名字，核心按请求的语言选好。
    pub title: String,
    /// 必填项，清除前要另填一个。
    pub required: bool,
    /// 填过。可选项清空以后是 `false`。
    pub filled: bool,
    /// 最后一次填写或清空的时间；没动过时为空。
    #[serde(default)]
    pub changed_at_ms: Option<u64>,
}

/// 主人填的密钥值：密钥名到值的表。
///
/// 安装、升级和确认卡片的回答都带这张表。必填项缺了返回 400 `invalid_input` 并列出缺的名字；
/// 带上卡片或预览里没有的项也返回 400 `invalid_input`。
pub type SecretValues = HashMap<String, SecretString>;

/// [`secret_input_path`] 的请求体：新值；`value` 为空是清空可选项，必填项清空返回 400 `invalid_input`。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SecretUpdate {
    #[serde(default)]
    pub value: Option<SecretString>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn input() -> SecretInput {
        SecretInput {
            name: "brave_key".into(),
            title: "Brave Search API key".into(),
            description: Some("Free tier has 2,000 queries a month".into()),
            url: Some("https://api-dashboard.search.brave.com/app/keys".into()),
            required: true,
        }
    }

    /// 契约：一项密钥的 JSON 字段名是核心和 App 之间的接口，改了要两边一起改。
    #[test]
    fn the_wire_shape_is_stable() {
        assert_eq!(
            json!(input()),
            json!({
                "name": "brave_key",
                "title": "Brave Search API key",
                "description": "Free tier has 2,000 queries a month",
                "url": "https://api-dashboard.search.brave.com/app/keys",
                "required": true,
            })
        );

        let state = SecretInputState {
            name: "brave_key".into(),
            title: "Brave Search API key".into(),
            required: false,
            filled: true,
            changed_at_ms: Some(1_793_000_000_000),
        };
        assert_eq!(
            json!(state),
            json!({
                "name": "brave_key",
                "title": "Brave Search API key",
                "required": false,
                "filled": true,
                "changed_at_ms": 1_793_000_000_000u64,
            })
        );

        assert_eq!(
            json!(SecretUpdate {
                value: Some(SecretString::new("s3cret")),
            }),
            json!({"value": "s3cret"})
        );
        assert_eq!(json!(SecretUpdate { value: None }), json!({"value": null}));
        assert_eq!(
            secret_input_path("app.brave", "brave_key"),
            "/_wonton/api/plugins/app.brave/secrets/brave_key"
        );
    }

    /// 契约：值打进日志也不泄露，`Debug` 打码；表里的键照常显示。
    #[test]
    fn debug_hides_the_values() {
        let values = SecretValues::from([("brave_key".to_owned(), SecretString::new("s3cret"))]);
        assert_eq!(format!("{values:?}"), r#"{"brave_key": <redacted>}"#);
        assert_eq!(
            format!(
                "{:?}",
                SecretUpdate {
                    value: Some(SecretString::new("s3cret")),
                }
            ),
            "SecretUpdate { value: Some(<redacted>) }"
        );
    }

    /// 契约：旧核心没有这些字段，App 照常解析；旧 App 不带值，核心按没填算。
    #[test]
    fn a_side_without_the_new_fields_still_parses() {
        let input: SecretInput =
            serde_json::from_value(json!({"name": "a", "title": "T", "required": false})).unwrap();
        assert_eq!(input.description, None);
        assert_eq!(input.url, None);

        let state: SecretInputState = serde_json::from_value(json!({
            "name": "a",
            "title": "T",
            "required": false,
            "filled": false,
        }))
        .unwrap();
        assert_eq!(state.changed_at_ms, None);

        let update: SecretUpdate = serde_json::from_value(json!({})).unwrap();
        assert_eq!(update.value, None);
    }
}
