//! 设置 Schema 的解析和子集校验（ADR-0077）：只收画得出原生控件的写法，超出子集报 `settings_schema_invalid`。
//!
//! 这里只做结构，不看具体值；值由 [`super::value`] 校验。标题和说明按请求语言选好，选项和顺序照包里的 Schema。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use wonton_protocol::{SettingsSchemaProblem, SettingsSchemaProblemKind};

use super::super::languages::Languages;
use super::super::manifest::{LocalizedText, Manifest};
use crate::l10n;

/// 属性名最长 64，一份 Schema 最多 64 项。
pub(super) const MAX_FIELDS: usize = 64;
/// 属性名：小写字母开头，小写字母、数字和下划线。
const NAME: &str = r"^[a-z][a-z0-9_]*$";
const TITLE: &str = "x-wonton-title";
const DESCRIPTION: &str = "x-wonton-description";
const ENUM_TITLES: &str = "x-wonton-enum-titles";

/// 解析好的设置 Schema。
#[derive(Debug)]
pub(crate) struct Schema {
    /// 设置文件内容的 sha256，写请求带回来核对。
    pub(crate) digest: String,
    pub(crate) fields: Vec<Field>,
    pub(crate) required: BTreeSet<String>,
    /// `x-wonton-order` 列的显示顺序。
    pub(crate) order: Vec<String>,
}

/// 一项设置。
#[derive(Debug)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) title: LocalizedText,
    pub(crate) description: Option<LocalizedText>,
    pub(crate) kind: Kind,
    pub(crate) default: Option<Value>,
}

#[derive(Debug)]
pub(crate) enum Kind {
    Toggle,
    Integer {
        min: Option<i64>,
        max: Option<i64>,
    },
    Decimal {
        min: Option<String>,
        max: Option<String>,
    },
    Text {
        min_length: Option<u32>,
        max_length: Option<u32>,
        pattern: Option<String>,
    },
    Choice {
        values: Vec<String>,
        titles: BTreeMap<String, LocalizedText>,
        multiple: bool,
    },
}

impl Kind {
    /// 用来自己校验默认值的属性 Schema。
    fn single(&self) -> Value {
        match self {
            Self::Toggle => serde_json::json!({"type": "boolean"}),
            Self::Integer { min, max } => {
                let mut schema = serde_json::json!({"type": "integer"});
                if let Some(min) = min {
                    schema["minimum"] = (*min).into();
                }
                if let Some(max) = max {
                    schema["maximum"] = (*max).into();
                }
                schema
            }
            Self::Decimal { min, max } => {
                let mut schema = serde_json::json!({"type": "number"});
                if let Some(min) = min {
                    schema["minimum"] = serde_json::json!(to_number(min));
                }
                if let Some(max) = max {
                    schema["maximum"] = serde_json::json!(to_number(max));
                }
                schema
            }
            Self::Text {
                min_length,
                max_length,
                pattern,
            } => {
                let mut schema = serde_json::json!({"type": "string"});
                if let Some(min) = min_length {
                    schema["minLength"] = (*min).into();
                }
                if let Some(max) = max_length {
                    schema["maxLength"] = (*max).into();
                }
                if let Some(pattern) = pattern {
                    schema["pattern"] = pattern.clone().into();
                }
                schema
            }
            Self::Choice {
                values, multiple, ..
            } => {
                let items = serde_json::json!({"type": "string", "enum": values});
                if *multiple {
                    serde_json::json!({"type": "array", "items": items, "uniqueItems": true})
                } else {
                    items
                }
            }
        }
    }
}

fn to_number(raw: &str) -> f64 {
    raw.parse().unwrap_or_default()
}

/// 解析并校验设置文件。不合格时返回全部问题，按属性顺序。
pub(super) fn parse(
    manifest: &Manifest,
    bytes: &[u8],
    langs: &Languages,
) -> Result<Schema, Vec<SettingsSchemaProblem>> {
    let mut problems = Vec::new();
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            problems.push(SettingsSchemaProblem {
                name: None,
                kind: SettingsSchemaProblemKind::InvalidJson,
                message: l10n::text(langs, "plugin-settings-schema-invalid-json", &[]),
            });
            return Err(problems);
        }
    };
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            problems.push(SettingsSchemaProblem {
                name: None,
                kind: SettingsSchemaProblemKind::InvalidJson,
                message: l10n::text(langs, "plugin-settings-schema-invalid-json", &[]),
            });
            return Err(problems);
        }
    };
    let Some(root) = value.as_object() else {
        problems.push(root_problem(langs, "plugin-settings-schema-object"));
        return Err(problems);
    };
    check_root(root, langs, &mut problems);
    let Some(properties) = root.get("properties").and_then(Value::as_object) else {
        if !problems.iter().any(|p| p.name.is_none()) {
            problems.push(root_problem(langs, "plugin-settings-schema-properties"));
        }
        return Err(problems);
    };
    if properties.len() > MAX_FIELDS {
        problems.push(root_problem(langs, "plugin-settings-schema-too-many"));
    }
    let required = required_names(root, properties, langs, &mut problems);
    let mut fields = Vec::new();
    for (name, schema) in properties {
        let field = match parse_field(manifest, name, schema, required.contains(name), langs) {
            Ok(field) => field,
            Err(problem) => {
                problems.push(problem);
                continue;
            }
        };
        fields.push(field);
    }
    let order = order(root, properties, langs, &mut problems);
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Schema {
        digest: format!("sha256:{}", super::super::package_files::sha256_hex(bytes)),
        fields,
        required,
        order,
    })
}

fn root_problem(langs: &Languages, id: &'static str) -> SettingsSchemaProblem {
    SettingsSchemaProblem {
        name: None,
        kind: SettingsSchemaProblemKind::InvalidSchema,
        message: l10n::text(langs, id, &[]),
    }
}

fn check_root(
    root: &Map<String, Value>,
    langs: &Languages,
    problems: &mut Vec<SettingsSchemaProblem>,
) {
    for key in root.keys() {
        let allowed = matches!(
            key.as_str(),
            "$schema"
                | "type"
                | "properties"
                | "required"
                | "additionalProperties"
                | "x-wonton-order"
        );
        if !allowed {
            problems.push(unsupported(langs, None, key));
        }
    }
    match root.get("$schema").and_then(Value::as_str) {
        Some("https://json-schema.org/draft/2020-12/schema") => {}
        _ => problems.push(root_problem(langs, "plugin-settings-schema-draft")),
    }
    match root.get("type").and_then(Value::as_str) {
        Some("object") => {}
        _ => problems.push(root_problem(langs, "plugin-settings-schema-object")),
    }
    match root.get("additionalProperties") {
        Some(Value::Bool(false)) => {}
        _ => problems.push(root_problem(langs, "plugin-settings-schema-additional")),
    }
}

fn unsupported(langs: &Languages, name: Option<&str>, key: &str) -> SettingsSchemaProblem {
    SettingsSchemaProblem {
        name: name.map(str::to_owned),
        kind: SettingsSchemaProblemKind::UnsupportedKeyword,
        message: l10n::text(
            langs,
            "plugin-settings-schema-unsupported",
            &[("key", key.into())],
        ),
    }
}

fn required_names(
    root: &Map<String, Value>,
    properties: &Map<String, Value>,
    langs: &Languages,
    problems: &mut Vec<SettingsSchemaProblem>,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    match root.get("required") {
        None => {}
        Some(Value::Array(list)) => {
            for item in list {
                match item.as_str() {
                    Some(name) if properties.contains_key(name) => {
                        if !names.insert(name.to_owned()) {
                            problems.push(invalid_keyword(
                                langs,
                                Some(name),
                                "plugin-settings-schema-required-duplicate",
                            ));
                        }
                    }
                    Some(name) => problems.push(invalid_keyword(
                        langs,
                        Some(name),
                        "plugin-settings-schema-required-unknown",
                    )),
                    None => problems.push(invalid_keyword(
                        langs,
                        None,
                        "plugin-settings-schema-required",
                    )),
                }
            }
        }
        Some(_) => problems.push(invalid_keyword(
            langs,
            None,
            "plugin-settings-schema-required",
        )),
    }
    names
}

fn invalid_keyword(
    langs: &Languages,
    name: Option<&str>,
    id: &'static str,
) -> SettingsSchemaProblem {
    SettingsSchemaProblem {
        name: name.map(str::to_owned),
        kind: SettingsSchemaProblemKind::InvalidKeyword,
        message: l10n::text(langs, id, &[]),
    }
}

fn order(
    root: &Map<String, Value>,
    properties: &Map<String, Value>,
    langs: &Languages,
    problems: &mut Vec<SettingsSchemaProblem>,
) -> Vec<String> {
    let Some(value) = root.get("x-wonton-order") else {
        return Vec::new();
    };
    let Some(list) = value.as_array() else {
        problems.push(invalid_keyword(langs, None, "plugin-settings-schema-order"));
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for item in list {
        match item.as_str() {
            Some(name) if properties.contains_key(name) && seen.insert(name) => {
                order.push(name.to_owned())
            }
            _ => problems.push(invalid_keyword(langs, None, "plugin-settings-schema-order")),
        }
    }
    order
}

fn parse_field(
    manifest: &Manifest,
    name: &str,
    value: &Value,
    required: bool,
    langs: &Languages,
) -> Result<Field, SettingsSchemaProblem> {
    let Some(schema) = value.as_object() else {
        return Err(invalid_keyword(
            langs,
            Some(name),
            "plugin-settings-schema-field",
        ));
    };
    if !regex::Regex::new(NAME).expect("固定正则").is_match(name) || name.len() > 64 {
        return Err(SettingsSchemaProblem {
            name: Some(name.to_owned()),
            kind: SettingsSchemaProblemKind::InvalidName,
            message: l10n::text(langs, "plugin-settings-schema-name", &[]),
        });
    }
    if manifest.secrets.contains_key(name) {
        return Err(invalid_keyword(
            langs,
            Some(name),
            "plugin-settings-schema-secret-name",
        ));
    }
    if sensitive(schema) {
        return Err(SettingsSchemaProblem {
            name: Some(name.to_owned()),
            kind: SettingsSchemaProblemKind::Sensitive,
            message: l10n::text(langs, "plugin-settings-schema-sensitive", &[]),
        });
    }
    let title = localized(
        schema.get(TITLE),
        langs,
        name,
        SettingsSchemaProblemKind::MissingTitle,
        "plugin-settings-schema-title",
    )?;
    let description = match schema.get(DESCRIPTION) {
        Some(value) => Some(localized(
            Some(value),
            langs,
            name,
            SettingsSchemaProblemKind::InvalidKeyword,
            "plugin-settings-schema-description",
        )?),
        None => None,
    };
    let kind = parse_kind(schema, langs, name)?;
    for key in schema.keys() {
        let allowed = matches!(
            key.as_str(),
            "type"
                | "enum"
                | "items"
                | "minimum"
                | "maximum"
                | "minLength"
                | "maxLength"
                | "pattern"
                | "default"
                | "uniqueItems"
                | TITLE
                | DESCRIPTION
                | ENUM_TITLES
        );
        if !allowed {
            return Err(unsupported(langs, Some(name), key));
        }
    }
    let default = match schema.get("default") {
        Some(value) => {
            check_default(value, &kind, langs, name)?;
            Some(value.clone())
        }
        None => None,
    };
    let _ = required;
    Ok(Field {
        name: name.to_owned(),
        title,
        description,
        kind,
        default,
    })
}

fn sensitive(schema: &Map<String, Value>) -> bool {
    matches!(schema.get("writeOnly"), Some(Value::Bool(true)))
        || matches!(schema.get("x-wonton-secret"), Some(Value::Bool(true)))
        || schema.get("format").and_then(Value::as_str) == Some("password")
}

fn localized(
    value: Option<&Value>,
    langs: &Languages,
    name: &str,
    kind: SettingsSchemaProblemKind,
    id: &'static str,
) -> Result<LocalizedText, SettingsSchemaProblem> {
    let Some(value) = value else {
        return Err(SettingsSchemaProblem {
            name: Some(name.to_owned()),
            kind,
            message: l10n::text(langs, id, &[]),
        });
    };
    let Some(map) = value.as_object() else {
        return Err(SettingsSchemaProblem {
            name: Some(name.to_owned()),
            kind: SettingsSchemaProblemKind::InvalidKeyword,
            message: l10n::text(langs, id, &[]),
        });
    };
    let mut text = LocalizedText::new();
    for (lang, value) in map {
        if let Some(value) = value.as_str() {
            text.insert(lang.clone(), value.to_owned());
        } else {
            text.insert(lang.clone(), String::new());
        }
    }
    if text.values().all(String::is_empty) {
        return Err(SettingsSchemaProblem {
            name: Some(name.to_owned()),
            kind: SettingsSchemaProblemKind::MissingTitle,
            message: l10n::text(langs, id, &[]),
        });
    }
    Ok(text)
}

fn parse_kind(
    schema: &Map<String, Value>,
    langs: &Languages,
    name: &str,
) -> Result<Kind, SettingsSchemaProblem> {
    let Some(Value::String(kind)) = schema.get("type") else {
        return Err(invalid_keyword(
            langs,
            Some(name),
            "plugin-settings-schema-type",
        ));
    };
    match kind.as_str() {
        "boolean" => Ok(Kind::Toggle),
        "integer" => Ok(Kind::Integer {
            min: integer(schema.get("minimum"), langs, name)?,
            max: integer(schema.get("maximum"), langs, name)?,
        }),
        "number" => Ok(Kind::Decimal {
            min: decimal(schema.get("minimum"), langs, name)?,
            max: decimal(schema.get("maximum"), langs, name)?,
        }),
        "string" => match schema.get("enum") {
            Some(Value::Array(values)) => single_choice(schema, values, langs, name),
            Some(_) => Err(invalid_keyword(
                langs,
                Some(name),
                "plugin-settings-schema-enum",
            )),
            None => Ok(Kind::Text {
                min_length: length(schema.get("minLength"), langs, name)?,
                max_length: length(schema.get("maxLength"), langs, name)?,
                pattern: match schema.get("pattern") {
                    Some(Value::String(pattern)) => {
                        regex::Regex::new(pattern).map_err(|_| {
                            invalid_keyword(langs, Some(name), "plugin-settings-schema-pattern")
                        })?;
                        Some(pattern.clone())
                    }
                    Some(_) => {
                        return Err(invalid_keyword(
                            langs,
                            Some(name),
                            "plugin-settings-schema-pattern",
                        ));
                    }
                    None => None,
                },
            }),
        },
        "array" => multiple_choice(schema, langs, name),
        _ => Err(unsupported(langs, Some(name), "type")),
    }
}

fn integer(
    value: Option<&Value>,
    langs: &Languages,
    name: &str,
) -> Result<Option<i64>, SettingsSchemaProblem> {
    match value {
        None => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| invalid_keyword(langs, Some(name), "plugin-settings-schema-bound")),
    }
}

fn decimal(
    value: Option<&Value>,
    langs: &Languages,
    name: &str,
) -> Result<Option<String>, SettingsSchemaProblem> {
    match value {
        None => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .map(|number| Some(number.to_string()))
            .ok_or_else(|| invalid_keyword(langs, Some(name), "plugin-settings-schema-bound")),
    }
}

fn length(
    value: Option<&Value>,
    langs: &Languages,
    name: &str,
) -> Result<Option<u32>, SettingsSchemaProblem> {
    match value {
        None => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| invalid_keyword(langs, Some(name), "plugin-settings-schema-length")),
    }
}

fn single_choice(
    schema: &Map<String, Value>,
    values: &[Value],
    langs: &Languages,
    name: &str,
) -> Result<Kind, SettingsSchemaProblem> {
    let values = strings(values, langs, name)?;
    let titles = enum_titles(schema.get(ENUM_TITLES), &values, langs, name)?;
    Ok(Kind::Choice {
        values,
        titles,
        multiple: false,
    })
}

fn multiple_choice(
    schema: &Map<String, Value>,
    langs: &Languages,
    name: &str,
) -> Result<Kind, SettingsSchemaProblem> {
    let Some(Value::Object(items)) = schema.get("items") else {
        return Err(invalid_keyword(
            langs,
            Some(name),
            "plugin-settings-schema-items",
        ));
    };
    if items.get("type").and_then(Value::as_str) != Some("string") {
        return Err(invalid_keyword(
            langs,
            Some(name),
            "plugin-settings-schema-items",
        ));
    }
    for key in items.keys() {
        if !matches!(key.as_str(), "type" | "enum") {
            return Err(unsupported(langs, Some(name), key));
        }
    }
    if schema.get("uniqueItems") != Some(&Value::Bool(true)) {
        return Err(invalid_keyword(
            langs,
            Some(name),
            "plugin-settings-schema-unique",
        ));
    }
    let Some(Value::Array(values)) = items.get("enum") else {
        return Err(invalid_keyword(
            langs,
            Some(name),
            "plugin-settings-schema-enum",
        ));
    };
    let values = strings(values, langs, name)?;
    let titles = enum_titles(schema.get(ENUM_TITLES), &values, langs, name)?;
    Ok(Kind::Choice {
        values,
        titles,
        multiple: true,
    })
}

fn strings(
    values: &[Value],
    langs: &Languages,
    name: &str,
) -> Result<Vec<String>, SettingsSchemaProblem> {
    if values.is_empty() {
        return Err(invalid_keyword(
            langs,
            Some(name),
            "plugin-settings-schema-enum",
        ));
    }
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let Some(value) = value.as_str() else {
            return Err(invalid_keyword(
                langs,
                Some(name),
                "plugin-settings-schema-enum",
            ));
        };
        if out.iter().any(|seen| seen == value) {
            return Err(invalid_keyword(
                langs,
                Some(name),
                "plugin-settings-schema-enum-duplicate",
            ));
        }
        out.push(value.to_owned());
    }
    Ok(out)
}

fn enum_titles(
    value: Option<&Value>,
    values: &[String],
    langs: &Languages,
    name: &str,
) -> Result<BTreeMap<String, LocalizedText>, SettingsSchemaProblem> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let Some(map) = value.as_object() else {
        return Err(invalid_keyword(
            langs,
            Some(name),
            "plugin-settings-schema-enum-titles",
        ));
    };
    let mut titles = BTreeMap::new();
    for key in map.keys() {
        if !values.iter().any(|value| value == key) {
            return Err(invalid_keyword(
                langs,
                Some(name),
                "plugin-settings-schema-enum-titles",
            ));
        }
    }
    for value in values {
        let Some(title) = map.get(value) else {
            continue;
        };
        titles.insert(
            value.clone(),
            localized(
                Some(title),
                langs,
                name,
                SettingsSchemaProblemKind::InvalidKeyword,
                "plugin-settings-schema-enum-titles",
            )?,
        );
    }
    Ok(titles)
}

fn check_default(
    value: &Value,
    kind: &Kind,
    langs: &Languages,
    name: &str,
) -> Result<(), SettingsSchemaProblem> {
    let schema = kind.single();
    if !jsonschema::draft202012::is_valid(&schema, value) {
        return Err(SettingsSchemaProblem {
            name: Some(name.to_owned()),
            kind: SettingsSchemaProblemKind::InvalidDefault,
            message: l10n::text(langs, "plugin-settings-schema-default", &[]),
        });
    }
    Ok(())
}
