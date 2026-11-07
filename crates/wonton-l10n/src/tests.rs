use std::sync::Arc;

use super::{Catalog, check_catalog};

static RESOURCES: &[(&str, &str)] = &[
    (
        "en",
        "language = en\nhello = Hello, { $name }\nfiles = { $count ->\n    [one] One file\n   *[other] { $count } files\n}\n",
    ),
    ("zh-Hans", "language = zh-Hans\n"),
    ("zh-Hant", "language = zh-Hant\n"),
    ("ja", "language = ja\nhello = こんにちは、{ $name }さん\n"),
];

#[test]
fn lookup_uses_cldr_scripts_and_request_priority() {
    let catalog = Catalog::new(RESOURCES);
    for (requested, expected) in [
        ("zh-TW", "zh-Hant"),
        ("zh-HK", "zh-Hant"),
        ("zh-MO", "zh-Hant"),
        ("zh-SG", "zh-Hans"),
        ("zh-CN", "zh-Hans"),
        ("fr", "en"),
        ("ja-JP", "ja"),
    ] {
        let lang = catalog.negotiate(&[requested]);
        assert_eq!(catalog.format(lang, "language", &[]), expected);
        assert_eq!(catalog.tag(lang), expected);
    }
    let lang = catalog.negotiate(&["ja", "zh-TW"]);
    assert_eq!(catalog.format(lang, "language", &[]), "ja");
    let lang = catalog.negotiate(&["fr-FR", "ja"]);
    assert_eq!(catalog.format(lang, "language", &[]), "ja");
    let requested: [&str; 0] = [];
    let lang = catalog.negotiate(&requested);
    assert_eq!(catalog.format(lang, "language", &[]), "en");
}
#[test]
fn integers_select_english_plural_forms() {
    let catalog = Catalog::new(RESOURCES);
    let lang = catalog.negotiate(&["en"]);
    assert_eq!(crate::l10n!(catalog, lang, "files", count = 1), "One file");
    assert_eq!(
        crate::l10n!(catalog, lang, "files", count = 0_u64),
        "0 files"
    );
    assert_eq!(
        crate::l10n!(catalog, lang, "files", count = 2_usize),
        "2 files"
    );
}

#[test]
fn number_builtin_selects_ordinal_forms() {
    static RESOURCES: &[(&str, &str)] = &[(
        "en",
        "rank = { NUMBER($n, type: \"ordinal\") ->\n    [one] { $n }st\n    [two] { $n }nd\n    [few] { $n }rd\n   *[other] { $n }th\n}\n",
    )];
    let catalog = Catalog::new(RESOURCES);
    let lang = catalog.negotiate(&["en"]);
    for (n, expected) in [
        (1, "1st"),
        (2, "2nd"),
        (3, "3rd"),
        (11, "11th"),
        (22, "22nd"),
    ] {
        assert_eq!(crate::l10n!(catalog, lang, "rank", n = n), expected);
    }
}

#[test]
fn missing_messages_fall_back_to_english_then_id() {
    let catalog = Catalog::new(RESOURCES);
    let lang = catalog.negotiate(&["zh-TW"]);
    assert_eq!(
        crate::l10n!(catalog, lang, "hello", name = "A/B: 503"),
        "Hello, A/B: 503"
    );
    assert_eq!(catalog.format(lang, "unknown", &[]), "unknown");
    assert_eq!(crate::l10n!(catalog, lang, "files", count = 1), "One file");
}

#[test]
fn bundles_can_format_concurrently_without_isolation_marks() {
    let catalog = Arc::new(Catalog::new(RESOURCES));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let catalog = catalog.clone();
            std::thread::spawn(move || {
                let lang = catalog.negotiate(&["ja"]);
                crate::l10n!(catalog, lang, "hello", name = String::from("Alex"))
            })
        })
        .collect();
    for worker in workers {
        assert_eq!(worker.join().unwrap(), "こんにちは、Alexさん");
    }
}

#[test]
fn consistency_checks_all_messages_and_variables_in_attributes_and_selectors() {
    let base = "message = { $count ->\n    [one] { $name }\n   *[other] { $name }\n}\n    .detail = { NUMBER($size) }\nother = Other\n";
    assert!(check_catalog(&[("en", base), ("ja", base)]).is_ok());
    for other in [
        base.replace("other = Other\n", ""),
        base.replace("$name", "$user"),
        base.replace("$count", "$total"),
        base.replace("$size", "$bytes"),
        format!("{base}extra = Extra\n"),
    ] {
        assert!(check_catalog(&[("en", base), ("ja", &other)]).is_err());
    }
}

#[test]
fn consistency_reports_invalid_syntax_duplicate_entries_and_bad_languages() {
    for resources in [
        vec![],
        vec![("ja", "message = Hi\n")],
        vec![("en", "message = {\n"), ("ja", "message = Hi\n")],
        vec![("en", "message = Hi\n"), ("ja", "message = {\n")],
        vec![("en", "message = Hi\nmessage = Again\n")],
        vec![("en", "message = Hi\n"), ("bad_tag", "message = Hi\n")],
        vec![("en", "message = Hi\n"), ("en", "message = Hi\n")],
    ] {
        assert!(check_catalog(&resources).is_err());
    }
}
