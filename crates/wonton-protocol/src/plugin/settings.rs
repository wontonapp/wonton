//! 插件设置（ADR-0077）：清单里的 JSON Schema 就是设置页的形状。核心按 Schema 校验值、把值存进插件记录，改了设置只
//! 重建渲染结果里用到它的服务。敏感项不在这里，走 ADR-0064 的输入密钥。
//!
//! 设置页由壳照 [`PluginSettingsView`] 画原生控件。不认识的类型落到 [`PluginSettingKind::Other`]，壳画 JSON 文本编辑框，
//! 核心照样按 Schema 校验。文字由核心按请求的语言选好，壳原样显示。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 读、写设置页的接口（`GET`、`PUT`）。
pub fn plugin_settings_path(plugin_id: &str) -> String {
    format!("/_wonton/api/plugins/{plugin_id}/settings")
}

/// 只校验、不保存的接口（`POST`）：请求体同 [`PluginSettingsUpdate`]，响应 [`PluginSettingsView`]。
pub fn plugin_settings_preview_path(plugin_id: &str) -> String {
    format!("/_wonton/api/plugins/{plugin_id}/settings/preview")
}

/// 一个设置的值。类型和 [`PluginSettingKind`] 对应；[`SettingValue::Json`] 用在 App 不认识的字段上。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingValue {
    Flag {
        value: bool,
    },
    Integer {
        value: i64,
    },
    /// 十进制写法，如 `1.5`：字符串保证跨端不丢精度。
    Decimal {
        value: String,
    },
    Text {
        value: String,
    },
    Texts {
        value: Vec<String>,
    },
    /// JSON 文本，核心解析后按 Schema 校验。
    Json {
        value: String,
    },
    /// 比这个 App 新的核心给的值类型，壳按空值显示。
    #[serde(other)]
    Other,
}

/// 一个设置的值表：属性名到值。
pub type SettingValues = HashMap<String, SettingValue>;

/// `PUT` 和 `POST …/settings/preview` 的请求体：只带改动的项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginSettingsUpdate {
    /// 读接口给的 `schema_digest`。插件在中间换了设置文件时对不上，返回 409 `changed`。
    pub schema_digest: String,
    /// 属性名到新值；值是 `null` 时清空这一项、回到默认值。没出现的项不动。
    pub values: HashMap<String, Option<SettingValue>>,
}

/// 读设置页、保存、预览校验的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginSettingsView {
    /// 设置文件的内容摘要（`sha256:…`），写请求原样带回。
    pub schema_digest: String,
    pub form: SettingsForm,
    /// 最后一次保存的时间，Unix 毫秒；没改过时为空。
    #[serde(default)]
    pub changed_at_ms: Option<u64>,
}

/// 设置表单：安装、升级预览和确认卡片里带它，主人用同一套原生控件填。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettingsForm {
    /// 按插件包给的顺序。
    pub fields: Vec<PluginSettingField>,
}

/// 设置页的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginSettingField {
    /// 属性名（清单 `settings` 的 Schema 里的键），写请求的键，也是 `WONTON_SETTING_<名字大写>` 的来源。
    pub name: String,
    /// 控件的标题，插件包给的，核心按请求的语言选好。
    pub title: String,
    /// 一句补充；插件包没写时为空。
    #[serde(default)]
    pub description: Option<String>,
    /// 必填项：没有值、也没有默认值时提交不了。
    pub required: bool,
    pub kind: PluginSettingKind,
    /// 现在存着的值（升级预览里是装着版本的值）；没设过、用默认值时为空。
    #[serde(default)]
    pub value: Option<SettingValue>,
    /// Schema 给的默认值；没有默认值时为空。清空这一项时回到它。
    #[serde(default)]
    pub default: Option<SettingValue>,
    /// 渲染结果里出现这个设置的服务。改它只重建这些容器；为空时下次启动生效。
    #[serde(default)]
    pub services: Vec<String>,
    /// 当前值过不了 Schema 时的问题（插件升级改了 Schema 等），改好才能提交。
    #[serde(default)]
    pub problem: Option<SettingsFieldProblem>,
}

/// 字段该画成什么控件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginSettingKind {
    /// 开关。
    Toggle,
    /// 整数输入，带范围。
    Integer {
        #[serde(default)]
        min: Option<i64>,
        #[serde(default)]
        max: Option<i64>,
    },
    /// 小数输入，范围是十进制写法。
    Decimal {
        #[serde(default)]
        min: Option<String>,
        #[serde(default)]
        max: Option<String>,
    },
    /// 文本输入。
    Text {
        #[serde(default)]
        min_length: Option<u32>,
        #[serde(default)]
        max_length: Option<u32>,
        /// JSON Schema 的正则，核心照它校验。
        #[serde(default)]
        pattern: Option<String>,
    },
    /// 从选项里挑，`multiple` 时是多选（值是 [`SettingValue::Texts`]）。
    Choice {
        options: Vec<PluginSettingOption>,
        multiple: bool,
    },
    /// 核心认、App 不认识的类型：画成 JSON 文本编辑框，值用 [`SettingValue::Json`]。
    #[serde(other)]
    Other,
}

/// 选项里的一个值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginSettingOption {
    pub value: SettingValue,
    /// 插件包给这个选项的标题，核心按请求的语言选好。
    pub title: String,
}

/// 一个字段当前的问题。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettingsFieldProblem {
    pub kind: SettingsFieldProblemKind,
    /// 按请求语言写的一句话。
    pub message: String,
}

/// 字段的问题类别，壳按它决定控件旁边显示什么。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SettingsFieldProblemKind {
    /// 必填，但没有值，也没有默认值。
    Missing,
    WrongType,
    BelowMinimum,
    AboveMaximum,
    TooShort,
    TooLong,
    Pattern,
    NotInOptions,
    DuplicateItems,
    /// JSON 文本解析不了。
    InvalidJson,
    /// Schema 里没有这个属性。
    UnknownField,
    /// 比这个 App 新的核心给的原因。
    #[serde(other)]
    Other,
}

/// `settings_invalid` 错误响应的额外字段（400）：一个或多个值过不了 Schema，原值不动、不重建。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettingsInvalidError {
    pub fields: Vec<SettingsFieldError>,
}

/// 一个没通过的字段。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettingsFieldError {
    /// 属性名。
    pub name: String,
    pub kind: SettingsFieldProblemKind,
    /// 按请求语言写的一句话。
    pub message: String,
}

/// `settings_schema_invalid` 错误响应的额外字段（400）：插件包里的 Schema 自己不合格，安装、升级在动资源之前失败。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettingsSchemaError {
    pub problems: Vec<SettingsSchemaProblem>,
}

/// Schema 的一个问题，位置是属性名；根上的问题为空。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettingsSchemaProblem {
    #[serde(default)]
    pub name: Option<String>,
    pub kind: SettingsSchemaProblemKind,
    /// 按请求语言写的一句话。
    pub message: String,
}

/// Schema 的问题类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SettingsSchemaProblemKind {
    /// 文件不是合法 JSON。
    InvalidJson,
    /// 不是 Draft 2020-12 的对象 Schema，或根上不符合要求。
    InvalidSchema,
    /// 用到了这版处理不了的写法（如远程 `$ref`）。
    UnsupportedKeyword,
    /// 属性没有 `x-wonton-title`。
    MissingTitle,
    /// 属性是敏感项（`writeOnly`、`format: password`、`x-wonton-secret`），要移到输入密钥。
    Sensitive,
    /// 默认值自己过不了属性 Schema。
    InvalidDefault,
    /// 关键字写法不对，如空枚举、重复的枚举值、颠倒的上下限。
    InvalidKeyword,
    /// 属性名不合规、属性太多。
    InvalidName,
    /// 比这个 App 新的核心给的原因。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn field() -> PluginSettingField {
        PluginSettingField {
            name: "months".into(),
            title: "显示几个月".into(),
            description: Some("图表里的时间范围".into()),
            required: true,
            kind: PluginSettingKind::Integer {
                min: Some(1),
                max: Some(12),
            },
            value: Some(SettingValue::Integer { value: 3 }),
            default: Some(SettingValue::Integer { value: 3 }),
            services: vec!["server".into()],
            problem: None,
        }
    }

    /// 契约：设置页的 JSON 字段名是核心和 App 之间的接口，改了要两边一起改。
    #[test]
    fn the_wire_shape_is_stable() {
        assert_eq!(
            plugin_settings_path("app.actual"),
            "/_wonton/api/plugins/app.actual/settings"
        );
        assert_eq!(
            plugin_settings_preview_path("app.actual"),
            "/_wonton/api/plugins/app.actual/settings/preview"
        );

        assert_eq!(
            json!(field()),
            json!({
                "name": "months",
                "title": "显示几个月",
                "description": "图表里的时间范围",
                "required": true,
                "kind": {"type": "integer", "min": 1, "max": 12},
                "value": {"type": "integer", "value": 3},
                "default": {"type": "integer", "value": 3},
                "services": ["server"],
                "problem": null,
            })
        );

        let view = PluginSettingsView {
            schema_digest: "sha256:ab".into(),
            form: SettingsForm {
                fields: vec![PluginSettingField {
                    kind: PluginSettingKind::Choice {
                        options: vec![
                            PluginSettingOption {
                                value: SettingValue::Text {
                                    value: "household".into(),
                                },
                                title: "家庭".into(),
                            },
                            PluginSettingOption {
                                value: SettingValue::Text {
                                    value: "personal".into(),
                                },
                                title: "个人".into(),
                            },
                        ],
                        multiple: false,
                    },
                    problem: Some(SettingsFieldProblem {
                        kind: SettingsFieldProblemKind::NotInOptions,
                        message: "选一项".into(),
                    }),
                    ..field()
                }],
            },
            changed_at_ms: Some(1_793_000_000_000),
        };
        assert_eq!(
            json!(view),
            json!({
                "schema_digest": "sha256:ab",
                "form": {"fields": [{
                    "name": "months",
                    "title": "显示几个月",
                    "description": "图表里的时间范围",
                    "required": true,
                    "kind": {"type": "choice", "multiple": false, "options": [
                        {"value": {"type": "text", "value": "household"}, "title": "家庭"},
                        {"value": {"type": "text", "value": "personal"}, "title": "个人"},
                    ]},
                    "value": {"type": "integer", "value": 3},
                    "default": {"type": "integer", "value": 3},
                    "services": ["server"],
                    "problem": {"kind": "not_in_options", "message": "选一项"},
                }]},
                "changed_at_ms": 1_793_000_000_000u64,
            })
        );

        let update = PluginSettingsUpdate {
            schema_digest: "sha256:ab".into(),
            values: HashMap::from([
                (
                    "months".to_owned(),
                    Some(SettingValue::Integer { value: 6 }),
                ),
                ("budget".to_owned(), None),
            ]),
        };
        assert_eq!(
            json!(update),
            json!({
                "schema_digest": "sha256:ab",
                "values": {
                    "budget": null,
                    "months": {"type": "integer", "value": 6},
                },
            })
        );
    }

    /// 契约：错误响应的额外字段和类别。
    #[test]
    fn the_error_payloads_are_stable() {
        let invalid = SettingsInvalidError {
            fields: vec![SettingsFieldError {
                name: "months".into(),
                kind: SettingsFieldProblemKind::AboveMaximum,
                message: "最多 12".into(),
            }],
        };
        assert_eq!(
            json!({"error": {"kind": "settings_invalid", "message": "设置不合法", "fields": invalid}}),
            json!({"error": {"kind": "settings_invalid", "message": "设置不合法", "fields": {
                "fields": [{"name": "months", "kind": "above_maximum", "message": "最多 12"}],
            }}})
        );

        let schema = SettingsSchemaError {
            problems: vec![SettingsSchemaProblem {
                name: Some("brave_key".into()),
                kind: SettingsSchemaProblemKind::Sensitive,
                message: "敏感项要移到密钥".into(),
            }],
        };
        assert_eq!(
            json!(schema),
            json!({"problems": [{
                "name": "brave_key",
                "kind": "sensitive",
                "message": "敏感项要移到密钥",
            }]})
        );
        assert_eq!(
            json!(SettingsSchemaProblem {
                name: None,
                kind: SettingsSchemaProblemKind::InvalidJson,
                message: "不是 JSON".into(),
            }),
            json!({"name": null, "kind": "invalid_json", "message": "不是 JSON"})
        );
    }

    /// 契约：旧核心没有这些字段，App 照常解析；设置页里没设过值的项也能解析。
    #[test]
    fn a_side_without_the_new_fields_still_parses() {
        let field: PluginSettingField = serde_json::from_value(json!({
            "name": "budget",
            "title": "预算",
            "required": false,
            "kind": {"type": "text"},
        }))
        .unwrap();
        assert_eq!(field.description, None);
        assert_eq!(field.value, None);
        assert_eq!(field.default, None);
        assert!(field.services.is_empty());
        assert_eq!(field.problem, None);

        let view: PluginSettingsView = serde_json::from_value(json!({
            "schema_digest": "sha256:ab",
            "form": {"fields": []},
        }))
        .unwrap();
        assert_eq!(view.changed_at_ms, None);

        let update: PluginSettingsUpdate =
            serde_json::from_value(json!({"schema_digest": "sha256:ab", "values": {}})).unwrap();
        assert!(update.values.is_empty());
    }

    /// 契约：不认识的控件和值类型不弄丢整份视图。
    #[test]
    fn unknown_kinds_fall_back() {
        let kind: PluginSettingKind =
            serde_json::from_value(json!({"type": "file", "accept": ["image/png"]})).unwrap();
        assert_eq!(kind, PluginSettingKind::Other);

        let value: SettingValue = serde_json::from_value(json!({"type": "duration"})).unwrap();
        assert_eq!(value, SettingValue::Other);

        let problem: SettingsFieldProblemKind = serde_json::from_value(json!("too_many")).unwrap();
        assert_eq!(problem, SettingsFieldProblemKind::Other);
    }
}
