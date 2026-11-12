//! 设置值（ADR-0077）：App 的类型和 JSON 互转、默认值、逐项校验和大小上限。

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;
use wonton_protocol::{SettingValue, SettingsFieldError, SettingsFieldProblemKind, SettingsForm};

use super::super::languages::Languages;
use super::schema::{Field, Kind, Schema};
use crate::l10n;

/// 整份生效值序列化后的上限。
pub(crate) const MAX_VALUES_BYTES: usize = 64 << 10;
/// 单个字符串的上限。
pub(crate) const MAX_STRING_BYTES: usize = 8 << 10;
/// 一个字符串数组的上限。
pub(crate) const MAX_ITEMS: usize = 64;

pub(crate) type Values = BTreeMap<String, Value>;

/// App 给的值转成 JSON。`Other`（App 不认识）为 `None`。
pub(crate) fn from_setting(value: &SettingValue) -> Option<Value> {
    match value {
        SettingValue::Flag { value } => Some(Value::Bool(*value)),
        SettingValue::Integer { value } => Some((*value).into()),
        SettingValue::Decimal { value } => value.parse::<f64>().ok().map(Value::from),
        SettingValue::Text { value } => Some(Value::String(value.clone())),
        SettingValue::Texts { value } => Some(Value::Array(
            value.iter().cloned().map(Value::String).collect(),
        )),
        SettingValue::Other => None,
    }
}

/// 存着的 JSON 转回 App 的值类型；字段已经不在了也照转，让壳能看到旧值。
pub(crate) fn to_setting(value: &Value) -> SettingValue {
    match value {
        Value::Bool(value) => SettingValue::Flag { value: *value },
        Value::Number(number) => match number.as_i64() {
            Some(value) => SettingValue::Integer { value },
            None => SettingValue::Decimal {
                value: number.to_string(),
            },
        },
        Value::String(value) => SettingValue::Text {
            value: value.clone(),
        },
        Value::Array(items) => SettingValue::Texts {
            value: items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
        },
        _ => SettingValue::Other,
    }
}

/// 把 Schema 的默认值补进候选值。
pub(crate) fn defaults(schema: &Schema, values: &mut BTreeMap<String, Value>) {
    for field in &schema.fields {
        if let (None, Some(default)) = (values.get(&field.name), &field.default) {
            values.insert(field.name.clone(), default.clone());
        }
    }
}

/// 校验一份候选值：不认识的字段、类型和约束、必填和大小。返回按属性顺序的问题。
pub(crate) fn validate(
    schema: &Schema,
    values: &Values,
    langs: &Languages,
) -> Vec<SettingsFieldError> {
    let mut errors = Vec::new();
    for name in values.keys() {
        if !schema.fields.iter().any(|field| &field.name == name) {
            errors.push(field_error(
                name,
                SettingsFieldProblemKind::UnknownField,
                langs,
                "plugin-settings-value-unknown-field",
                &[],
            ));
        }
    }
    for field in &schema.fields {
        let value = values.get(&field.name);
        if value.is_none() && schema.required.contains(&field.name) {
            errors.push(field_error(
                &field.name,
                SettingsFieldProblemKind::Missing,
                langs,
                "plugin-settings-value-missing",
                &[],
            ));
            continue;
        }
        let Some(value) = value else {
            continue;
        };
        check_value(field, value, langs, &mut errors);
    }
    if errors.is_empty() {
        check_size(values, langs, &mut errors);
    }
    errors
}

fn field_error(
    name: &str,
    kind: SettingsFieldProblemKind,
    langs: &Languages,
    id: &'static str,
    args: &[(&str, wonton_l10n::Arg<'_>)],
) -> SettingsFieldError {
    SettingsFieldError {
        name: name.to_owned(),
        kind,
        message: l10n::text(langs, id, args),
    }
}

fn check_value(
    field: &Field,
    value: &Value,
    langs: &Languages,
    errors: &mut Vec<SettingsFieldError>,
) {
    let name = &field.name;
    let mut bad = |kind, id: &'static str, args: &[(&str, wonton_l10n::Arg<'_>)]| {
        errors.push(field_error(name, kind, langs, id, args));
    };
    match &field.kind {
        Kind::Toggle => {
            if !value.is_boolean() {
                bad(
                    SettingsFieldProblemKind::WrongType,
                    "plugin-settings-value-wrong-type",
                    &[("type", "boolean".into())],
                );
            }
        }
        Kind::Integer { min, max } => check_integer(value, *min, *max, &mut bad),
        Kind::Decimal { min, max } => {
            check_decimal(value, min.as_deref(), max.as_deref(), &mut bad)
        }
        Kind::Text {
            min_length,
            max_length,
            pattern,
        } => check_text(
            value,
            *min_length,
            *max_length,
            pattern.as_deref(),
            &mut bad,
        ),
        Kind::Choice {
            values, multiple, ..
        } => check_choice(value, values, *multiple, &mut bad),
    }
}

/// 整数要真是整数，min/max 都比了才算数。
fn check_integer<'a>(
    value: &'a Value,
    min: Option<i64>,
    max: Option<i64>,
    bad: &mut impl FnMut(SettingsFieldProblemKind, &'static str, &[(&'a str, wonton_l10n::Arg<'a>)]),
) {
    let Some(number) = value.as_i64() else {
        bad(
            SettingsFieldProblemKind::WrongType,
            "plugin-settings-value-wrong-type",
            &[("type", "integer".into())],
        );
        return;
    };
    if min.is_some_and(|min| number < min) {
        bad(
            SettingsFieldProblemKind::BelowMinimum,
            "plugin-settings-value-below-minimum",
            &[("min", min.unwrap().to_string().into())],
        );
    }
    if max.is_some_and(|max| number > max) {
        bad(
            SettingsFieldProblemKind::AboveMaximum,
            "plugin-settings-value-above-maximum",
            &[("max", max.unwrap().to_string().into())],
        );
    }
}

/// 小数要真是数，min/max 按字面量解析，解析不了就当没有。
fn check_decimal<'a>(
    value: &'a Value,
    min: Option<&'a str>,
    max: Option<&'a str>,
    bad: &mut impl FnMut(SettingsFieldProblemKind, &'static str, &[(&'a str, wonton_l10n::Arg<'a>)]),
) {
    let Some(number) = value.as_f64() else {
        bad(
            SettingsFieldProblemKind::WrongType,
            "plugin-settings-value-wrong-type",
            &[("type", "number".into())],
        );
        return;
    };
    if min.is_some_and(|min| number < min.parse().unwrap_or_default()) {
        bad(
            SettingsFieldProblemKind::BelowMinimum,
            "plugin-settings-value-below-minimum",
            &[("min", min.into())],
        );
    }
    if max.is_some_and(|max| number > max.parse().unwrap_or_default()) {
        bad(
            SettingsFieldProblemKind::AboveMaximum,
            "plugin-settings-value-above-maximum",
            &[("max", max.into())],
        );
    }
}

/// 文字要真是字符串，长度按字符数，pattern 是插件自己声明的正则。
fn check_text<'a>(
    value: &'a Value,
    min_length: Option<u32>,
    max_length: Option<u32>,
    pattern: Option<&'a str>,
    bad: &mut impl FnMut(SettingsFieldProblemKind, &'static str, &[(&'a str, wonton_l10n::Arg<'a>)]),
) {
    let Some(text) = value.as_str() else {
        bad(
            SettingsFieldProblemKind::WrongType,
            "plugin-settings-value-wrong-type",
            &[("type", "string".into())],
        );
        return;
    };
    let length = text.chars().count() as u32;
    if let Some(min) = min_length.filter(|min| length < *min) {
        bad(
            SettingsFieldProblemKind::TooShort,
            "plugin-settings-value-too-short",
            &[("min", min.to_string().into())],
        );
    }
    if let Some(max) = max_length.filter(|max| length > *max) {
        bad(
            SettingsFieldProblemKind::TooLong,
            "plugin-settings-value-too-long",
            &[("max", max.to_string().into())],
        );
    }
    if let Some(pattern) = pattern
        && let Ok(regex) = regex::Regex::new(pattern)
        && !regex.is_match(text)
    {
        bad(
            SettingsFieldProblemKind::Pattern,
            "plugin-settings-value-pattern",
            &[],
        );
    }
}

/// 选项里的值必须在清单列出的 values 里；多选还要去重、每项都是字符串。
fn check_choice<'a>(
    value: &'a Value,
    values: &'a [String],
    multiple: bool,
    bad: &mut impl FnMut(SettingsFieldProblemKind, &'static str, &[(&'a str, wonton_l10n::Arg<'a>)]),
) {
    if !multiple {
        if !value
            .as_str()
            .is_some_and(|text| values.iter().any(|value| value == text))
        {
            bad(
                SettingsFieldProblemKind::NotInOptions,
                "plugin-settings-value-not-in-options",
                &[],
            );
        }
        return;
    }
    let Some(items) = value.as_array() else {
        bad(
            SettingsFieldProblemKind::WrongType,
            "plugin-settings-value-wrong-type",
            &[("type", "array".into())],
        );
        return;
    };
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        match item.as_str() {
            Some(text) if !seen.insert(text) => {
                bad(
                    SettingsFieldProblemKind::DuplicateItems,
                    "plugin-settings-value-duplicate",
                    &[],
                );
            }
            Some(text) if !values.iter().any(|value| value == text) => {
                bad(
                    SettingsFieldProblemKind::NotInOptions,
                    "plugin-settings-value-not-in-options",
                    &[],
                );
            }
            Some(_) => {}
            None => bad(
                SettingsFieldProblemKind::WrongType,
                "plugin-settings-value-wrong-type",
                &[("type", "string".into())],
            ),
        }
    }
}

fn check_size(values: &Values, langs: &Languages, errors: &mut Vec<SettingsFieldError>) {
    let mut total = 0usize;
    for (name, value) in values {
        let bytes = serde_json::to_vec(value).map_or(0, |bytes| bytes.len());
        total += bytes + name.len();
        match value {
            Value::String(text) if text.len() > MAX_STRING_BYTES => errors.push(field_error(
                name,
                SettingsFieldProblemKind::TooLong,
                langs,
                "plugin-settings-value-string-too-big",
                &[("max", MAX_STRING_BYTES.to_string().into())],
            )),
            Value::Array(items) if items.len() > MAX_ITEMS => errors.push(field_error(
                name,
                SettingsFieldProblemKind::TooLong,
                langs,
                "plugin-settings-value-array-too-big",
                &[("max", MAX_ITEMS.to_string().into())],
            )),
            _ => {}
        }
    }
    if total > MAX_VALUES_BYTES {
        errors.push(field_error(
            "",
            SettingsFieldProblemKind::TooLong,
            langs,
            "plugin-settings-value-too-big",
            &[("max", MAX_VALUES_BYTES.to_string().into())],
        ));
    }
}

/// 按 Schema 的 `default` 和一组显式值拼出安装要用的值；必填项缺了也是问题。
pub(crate) fn initial(
    schema: &Schema,
    given: &HashMap<String, SettingValue>,
    langs: &Languages,
) -> Result<Values, Vec<SettingsFieldError>> {
    let mut values = BTreeMap::new();
    let mut errors = Vec::new();
    for (name, value) in given {
        match from_setting(value) {
            Some(value) => {
                values.insert(name.clone(), value);
            }
            None => errors.push(field_error(
                name,
                SettingsFieldProblemKind::Other,
                langs,
                "plugin-settings-value-unknown-type",
                &[],
            )),
        }
    }
    defaults(schema, &mut values);
    if errors.is_empty() {
        errors = validate(schema, &values, langs);
    }
    if errors.is_empty() {
        Ok(values)
    } else {
        Err(errors)
    }
}

/// 写请求叠在当前值上的结果。`null` 是清空这一项、回到默认值。
pub(crate) fn merged(
    schema: &Schema,
    current: &Values,
    update: &HashMap<String, Option<SettingValue>>,
    langs: &Languages,
) -> Result<Values, Vec<SettingsFieldError>> {
    let mut values: BTreeMap<String, Value> = current
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let mut errors = Vec::new();
    for (name, value) in update {
        match value {
            None => {
                values.remove(name);
            }
            Some(value) => match from_setting(value) {
                Some(value) => {
                    values.insert(name.clone(), value);
                }
                None => errors.push(field_error(
                    name,
                    SettingsFieldProblemKind::Other,
                    langs,
                    "plugin-settings-value-unknown-type",
                    &[],
                )),
            },
        }
    }
    defaults(schema, &mut values);
    if errors.is_empty() {
        errors = validate(schema, &values, langs);
    }
    if errors.is_empty() {
        Ok(values)
    } else {
        Err(errors)
    }
}

/// 表单里当前值过不了 Schema 的问题，按字段索引。
pub(crate) fn problems(
    values: &Values,
    langs: &Languages,
    schema: &Schema,
) -> BTreeMap<String, wonton_protocol::SettingsFieldProblem> {
    validate(schema, values, langs)
        .into_iter()
        .map(|error| {
            (
                error.name,
                wonton_protocol::SettingsFieldProblem {
                    kind: error.kind,
                    message: error.message,
                },
            )
        })
        .collect()
}

/// 注入用的变量值：字符串原样，布尔 `true`/`false`，数字十进制，多选是 JSON 数组文本。
pub(crate) fn vars(values: &Values) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Bool(value) => value.to_string(),
                Value::Number(number) => number.to_string(),
                Value::Array(_) => value.to_string(),
                _ => String::new(),
            };
            (super::setting_var(name), text)
        })
        .collect()
}

/// 字段顺序：先按 `x-wonton-order`，其余按属性名。
pub(crate) fn ordered_fields(schema: &Schema) -> Vec<&Field> {
    let mut fields: Vec<&Field> = schema
        .order
        .iter()
        .filter_map(|name| schema.fields.iter().find(|field| &field.name == name))
        .collect();
    for field in &schema.fields {
        if !fields.iter().any(|seen| seen.name == field.name) {
            fields.push(field);
        }
    }
    fields
}

/// 表单的形状（不含值和服务）：标题和选项标题按请求语言选好。
pub(crate) fn form(schema: &Schema, values: &Values, langs: &Languages) -> SettingsForm {
    let problems = problems(values, langs, schema);
    let fields = ordered_fields(schema)
        .into_iter()
        .map(|field| {
            let (kind, default) = kind(field, langs);
            wonton_protocol::PluginSettingField {
                name: field.name.clone(),
                title: langs.pick_owned(&field.title),
                description: field
                    .description
                    .as_ref()
                    .map(|text| langs.pick_owned(text)),
                required: schema.required.contains(&field.name),
                kind,
                value: values.get(&field.name).map(to_setting),
                default,
                services: Vec::new(),
                problem: problems.get(&field.name).cloned(),
            }
        })
        .collect();
    SettingsForm { fields }
}

fn kind(
    field: &Field,
    langs: &Languages,
) -> (wonton_protocol::PluginSettingKind, Option<SettingValue>) {
    use wonton_protocol::{PluginSettingKind as Out, PluginSettingOption};
    let default = field.default.as_ref().map(to_setting);
    let kind = match &field.kind {
        Kind::Toggle => Out::Toggle,
        Kind::Integer { min, max } => Out::Integer {
            min: *min,
            max: *max,
        },
        Kind::Decimal { min, max } => Out::Decimal {
            min: min.clone(),
            max: max.clone(),
        },
        Kind::Text {
            min_length,
            max_length,
            pattern,
        } => Out::Text {
            min_length: *min_length,
            max_length: *max_length,
            pattern: pattern.clone(),
        },
        Kind::Choice {
            values,
            titles,
            multiple,
        } => Out::Choice {
            options: values
                .iter()
                .map(|value| PluginSettingOption {
                    value: SettingValue::Text {
                        value: value.clone(),
                    },
                    title: titles
                        .get(value)
                        .map(|title| langs.pick_owned(title))
                        .unwrap_or_else(|| value.clone()),
                })
                .collect(),
            multiple: *multiple,
        },
    };
    (kind, default)
}
