//! 主人在 App 里查看插件密钥：`GET /_wonton/api/plugins/{id}/secrets` 列出，
//! `GET /_wonton/api/plugins/{id}/secrets/{name}` 取值。只有主人能调。
//!
//! 只收发清单里声明了 `show_owner` 的 `generate` 项；`collect` 项是插件交给核心的凭据，不在这里。
//! `label` 由核心按 `Accept-Language` 挑好语言（同 [`super`]）。App 默认收起，点开时才取值，
//! 取到的值不缓存，交给壳后不复读、不打日志（值是 [`SecretString`]，`Debug` 打码）。

use serde::{Deserialize, Serialize};

use crate::SecretString;

/// 列表接口的响应：这个插件声明了 `show_owner` 的密钥，按名字排序。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OwnerPluginSecrets {
    pub secrets: Vec<OwnerPluginSecret>,
}

/// 一项可查看的密钥。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OwnerPluginSecret {
    /// 密钥名（清单 `x-wonton.secrets` 的键），取值接口的 `{name}`。
    pub name: String,
    /// 主人界面上这一行的名字（清单里的 `show_owner.label`）。
    pub label: String,
}

/// 取值接口的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OwnerPluginSecretValue {
    /// 密钥的值，注入到插件容器里的就是这段文本。
    pub value: SecretString,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 响应的 JSON 字段名是核心和 App 之间的契约，改了要两边一起改。
    #[test]
    fn wire_shape() {
        let list = OwnerPluginSecrets {
            secrets: vec![OwnerPluginSecret {
                name: "admin_password".into(),
                label: "管理员密码".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&list).unwrap(),
            serde_json::json!({
                "secrets": [{"name": "admin_password", "label": "管理员密码"}]
            })
        );
        assert_eq!(
            serde_json::to_value(OwnerPluginSecret {
                name: "a".into(),
                label: "b".into(),
            })
            .unwrap(),
            serde_json::json!({"name": "a", "label": "b"})
        );

        let value: OwnerPluginSecretValue = serde_json::from_str(r#"{"value":"s3cret"}"#).unwrap();
        assert_eq!(value.value.expose(), "s3cret");
        // 值打进日志也不泄露：Debug 打码。
        assert_eq!(
            format!("{:?}", value),
            "OwnerPluginSecretValue { value: <redacted> }"
        );
    }
}
