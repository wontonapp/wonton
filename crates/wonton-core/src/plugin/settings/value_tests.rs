//! 设置值校验的单元测试：类型和约束逐类、大小上限、清空回默认、默认值填充。

use std::collections::{BTreeMap, HashMap};

use serde_json::json;
use wonton_protocol::{SettingValue, SettingsFieldError, SettingsFieldProblemKind};

use super::schema::{Field, Kind, Schema};
use super::value::{self, MAX_ITEMS, MAX_STRING_BYTES, Values};
use crate::plugin::languages::Languages;
use crate::plugin::manifest::LocalizedText;

fn field(name: &str, kind: Kind) -> Field {
    Field {
        name: name.to_owned(),
        title: LocalizedText::new(),
        description: None,
        kind,
        default: None,
    }
}

fn schema(fields: Vec<Field>, required: &[&str]) -> Schema {
    Schema {
        digest: "d".to_owned(),
        required: required.iter().map(|name| name.to_string()).collect(),
        order: fields.iter().map(|field| field.name.clone()).collect(),
        fields,
    }
}

fn errors(schema: &Schema, values: &Values) -> Vec<SettingsFieldError> {
    value::validate(schema, values, &Languages::default())
}

fn kinds(list: &[SettingsFieldError]) -> Vec<SettingsFieldProblemKind> {
    list.iter().map(|error| error.kind).collect()
}

#[test]
fn integer_checks_type_and_bounds() {
    let s = schema(
        vec![field(
            "n",
            Kind::Integer {
                min: Some(1),
                max: Some(12),
            },
        )],
        &[],
    );
    let mut values = Values::new();
    values.insert("n".to_owned(), json!("x"));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::WrongType]
    );
    values.insert("n".to_owned(), json!(0));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::BelowMinimum]
    );
    values.insert("n".to_owned(), json!(13));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::AboveMaximum]
    );
    values.insert("n".to_owned(), json!(12));
    assert!(errors(&s, &values).is_empty());
}

#[test]
fn decimal_bounds_parse_from_the_schema_text() {
    let s = schema(
        vec![field(
            "n",
            Kind::Decimal {
                min: Some("0.5".to_owned()),
                max: None,
            },
        )],
        &[],
    );
    let mut values = Values::new();
    values.insert("n".to_owned(), json!(0.4));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::BelowMinimum]
    );
    values.insert("n".to_owned(), json!(0.5));
    assert!(errors(&s, &values).is_empty());
}

#[test]
fn text_checks_length_and_pattern() {
    let s = schema(
        vec![field(
            "t",
            Kind::Text {
                min_length: Some(2),
                max_length: Some(4),
                pattern: Some("^[a-z]+$".to_owned()),
            },
        )],
        &[],
    );
    let mut values = Values::new();
    values.insert("t".to_owned(), json!("a"));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::TooShort]
    );
    values.insert("t".to_owned(), json!("abcde"));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::TooLong]
    );
    values.insert("t".to_owned(), json!("ab1"));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::Pattern]
    );
    values.insert("t".to_owned(), json!("ab"));
    assert!(errors(&s, &values).is_empty());
}

#[test]
fn choice_checks_members_and_duplicates() {
    let s = schema(
        vec![field(
            "c",
            Kind::Choice {
                values: vec!["a".to_owned(), "b".to_owned()],
                titles: BTreeMap::new(),
                multiple: false,
            },
        )],
        &[],
    );
    let mut values = Values::new();
    values.insert("c".to_owned(), json!("z"));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::NotInOptions]
    );
    values.insert("c".to_owned(), json!("a"));
    assert!(errors(&s, &values).is_empty());

    let multi = schema(
        vec![field(
            "m",
            Kind::Choice {
                values: vec!["a".to_owned(), "b".to_owned()],
                titles: BTreeMap::new(),
                multiple: true,
            },
        )],
        &[],
    );
    let mut values = Values::new();
    values.insert("m".to_owned(), json!(["a", "a"]));
    assert_eq!(
        kinds(&errors(&multi, &values)),
        vec![SettingsFieldProblemKind::DuplicateItems]
    );
    values.insert("m".to_owned(), json!(["a", "z"]));
    assert_eq!(
        kinds(&errors(&multi, &values)),
        vec![SettingsFieldProblemKind::NotInOptions]
    );
    values.insert("m".to_owned(), json!([1]));
    assert_eq!(
        kinds(&errors(&multi, &values)),
        vec![SettingsFieldProblemKind::WrongType]
    );
    values.insert("m".to_owned(), json!(["a", "b"]));
    assert!(errors(&multi, &values).is_empty());
}

#[test]
fn unknown_missing_and_required_are_reported() {
    let s = schema(
        vec![field("n", Kind::Toggle), field("r", Kind::Toggle)],
        &["r"],
    );
    let mut values = Values::new();
    values.insert("x".to_owned(), json!(true));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![
            SettingsFieldProblemKind::UnknownField,
            SettingsFieldProblemKind::Missing
        ]
    );
}

#[test]
fn oversized_strings_and_arrays_are_refused() {
    let s = schema(
        vec![
            field(
                "t",
                Kind::Text {
                    min_length: None,
                    max_length: None,
                    pattern: None,
                },
            ),
            field(
                "m",
                Kind::Choice {
                    values: (0..MAX_ITEMS + 1).map(|i| format!("v{i}")).collect(),
                    titles: BTreeMap::new(),
                    multiple: true,
                },
            ),
        ],
        &[],
    );
    let mut values = Values::new();
    values.insert("t".to_owned(), json!("x".repeat(MAX_STRING_BYTES + 1)));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::TooLong]
    );
    values.clear();
    // 65 个互不相同的项：没有别的问题时才查大小，报数组超 64 项。
    let items: Vec<String> = (0..MAX_ITEMS + 1).map(|i| format!("v{i}")).collect();
    values.insert("m".to_owned(), json!(items));
    assert_eq!(
        kinds(&errors(&s, &values)),
        vec![SettingsFieldProblemKind::TooLong]
    );
}

#[test]
fn null_clears_back_to_the_default() {
    let mut f = field(
        "n",
        Kind::Integer {
            min: None,
            max: None,
        },
    );
    f.default = Some(json!(3));
    let s = schema(vec![f], &[]);
    let mut current = Values::new();
    current.insert("n".to_owned(), json!(9));
    let mut update = HashMap::new();
    update.insert("n".to_owned(), None);
    let values = value::merged(&s, &current, &update, &Languages::default()).unwrap();
    assert_eq!(values.get("n"), Some(&json!(3)));
}

#[test]
fn initial_fills_defaults_and_keeps_explicit_values() {
    let mut f = field(
        "n",
        Kind::Integer {
            min: None,
            max: None,
        },
    );
    f.default = Some(json!(3));
    let s = schema(vec![f], &[]);
    let mut given = HashMap::new();
    given.insert("n".to_owned(), SettingValue::Integer { value: 7 });
    let values = value::initial(&s, &given, &Languages::default()).unwrap();
    assert_eq!(values.get("n"), Some(&json!(7)));
    let values = value::initial(&s, &HashMap::new(), &Languages::default()).unwrap();
    assert_eq!(values.get("n"), Some(&json!(3)));
}
