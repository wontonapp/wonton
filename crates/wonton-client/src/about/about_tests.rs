use super::*;

fn catalog() -> String {
    r#"{
      "generated_with": "cargo-about 0.9.3",
      "components": [
        {"source": "rust", "name": "serde", "version": "1.0.0", "license_ids": ["MIT", "Apache-2.0"]},
        {"source": "rust", "name": "opaque-debug", "version": "0.3.0", "license_ids": ["MIT"]},
        {"source": "android", "name": "androidx.core:core", "version": "1.15.0", "license_ids": ["Apache-2.0"]}
      ],
      "licenses": {
        "Apache-2.0": {"name": "Apache License 2.0", "text": "Apache text"},
        "MIT": {"name": "MIT License", "text": "MIT text"},
        "Zlib": {"name": "zlib License", "text": "zlib text"}
      }
    }"#
    .to_owned()
}

#[test]
fn groups_components_under_their_license() {
    let view = licenses(&catalog()).expect("清单可用");
    let names: Vec<&str> = view
        .licenses
        .iter()
        .map(|license| license.name.as_str())
        .collect();
    assert_eq!(names, ["Apache License 2.0", "MIT License"]);
    assert_eq!(view.licenses[1].text, "MIT text");
    assert_eq!(view.licenses[1].id, "MIT");
    let components: Vec<&str> = view.licenses[1]
        .components
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    // 一个组件用了两种许可就出现在两组里：MIT 这组按名字排，opaque-debug 在 serde 前面。
    assert_eq!(components, ["opaque-debug", "serde"]);
    assert_eq!(view.licenses[1].components[1].version, "1.0.0");
}

#[test]
fn drops_licenses_no_component_uses() {
    let view = licenses(&catalog()).expect("清单可用");
    assert!(
        view.licenses
            .iter()
            .all(|license| license.name != "zlib License")
    );
}

#[test]
fn rejects_a_list_none_of_whose_licenses_are_used() {
    let json = r#"{
      "components": [{"source": "rust", "name": "serde", "version": "1.0.0", "license_ids": []}],
      "licenses": {"MIT": {"name": "MIT", "text": "MIT text"}}
    }"#;
    let error = licenses(json).expect_err("没有可显示的许可要报错");
    assert!(
        matches!(error, CoreError::InvalidInput { ref reason } if !reason.is_empty()),
        "{error}"
    );
}

#[test]
fn rejects_license_without_text() {
    let json = catalog().replace("MIT text", " ");
    let error = licenses(&json).expect_err("没有全文要报错");
    assert!(
        matches!(error, CoreError::InvalidInput { ref reason } if !reason.is_empty()),
        "{error}"
    );
}

#[test]
fn rejects_a_list_without_components() {
    let error = licenses(r#"{"components": [], "licenses": {}}"#).expect_err("空清单要报错");
    assert!(
        matches!(error, CoreError::InvalidInput { ref reason } if !reason.is_empty()),
        "{error}"
    );
}

#[test]
fn rejects_broken_json() {
    let error = licenses("{").expect_err("解析失败要报错");
    assert!(
        matches!(error, CoreError::InvalidInput { ref reason } if !reason.is_empty()),
        "{error}"
    );
}

#[test]
fn privacy_covers_every_language() {
    for language in ["en", "zh-Hans", "zh-Hant", "ja"] {
        l10n::set_languages(&[language.to_owned()]);
        let view = privacy();
        assert_eq!(view.sections.len(), SECTIONS.len());
        for section in &view.sections {
            assert!(!section.title.is_empty(), "{language} 缺标题");
            assert!(!section.paragraphs.is_empty(), "{language} 缺段落");
            for paragraph in &section.paragraphs {
                assert!(
                    paragraph.chars().count() > 5,
                    "{language} 的 {paragraph:?} 太短"
                );
            }
        }
    }
}
