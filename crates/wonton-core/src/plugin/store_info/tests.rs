use super::super::InstallError;
use super::super::package::{PackageError, PluginPackage};
use super::super::package_files::{MAX_FILE_BYTES, MAX_PACKAGE_BYTES, PackageFiles};
use super::super::storefront::index::IndexEntry;
use super::next_spec;
use crate::l10n::CATALOG;

/// 校验失败的话按 `langs()`（默认英文）里的词条拼出来，测试不再钉住某一种语言的写法。
fn message(id: &str, args: &[(&str, &str)]) -> String {
    let lang = CATALOG.negotiate(&["en"]);
    CATALOG.format(
        lang,
        id,
        &args
            .iter()
            .map(|(k, v)| (*k, (*v).into()))
            .collect::<Vec<_>>(),
    )
}

fn without_spec(extra: &str) -> String {
    format!(
        "services:\n  web:\n    image: nginx\nx-wonton:\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {{zh: 测试, en: Test}}\n  description: {{zh: 测试, en: Test}}\n  permissions: []\n{extra}"
    )
}

fn langs() -> crate::plugin::languages::Languages {
    crate::plugin::languages::Languages::default()
}

fn compose(extra: &str) -> String {
    without_spec(&format!("  spec: '0.5'\n{extra}"))
}

fn parse_err(extra: &str) -> String {
    match PluginPackage::parse(&compose(extra), &langs()) {
        Ok(_) => panic!("应被拒绝：{extra}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn store_info_parses() {
    let yaml = compose(
        "  author: {name: Example, url: https://example.com}\n  \
         homepage: https://immich.app\n  license: AGPL-3.0-only\n  tags: {zh: [相册, AI], en: [Photos, AI]}\n  \
         about: {zh: 第一段, en: First}\n  release_notes: {zh: 修了问题, en: Fixes}\n  \
         screenshots: [shots/1.png, shots/2.jpg]",
    );
    let m = PluginPackage::parse(&yaml, &langs()).unwrap().manifest;
    assert_eq!(m.author.unwrap().name, "Example");
    assert_eq!(m.tags["en"], ["Photos", "AI"]);
    assert_eq!(m.screenshots.unwrap().len(), 2);
    let dual = compose("  license: MIT OR Apache-2.0");
    PluginPackage::parse(&dual, &langs()).unwrap();
    // 不在 SPDX 列表里的许可证用 LicenseRef，全文地址可选。
    let eula = compose(
        "  license: LicenseRef-n8n-Sustainable-Use-License\n  \
         license_url: https://docs.n8n.io/sustainable-use-license/",
    );
    let m = PluginPackage::parse(&eula, &langs()).unwrap().manifest;
    assert_eq!(
        m.license.as_deref(),
        Some("LicenseRef-n8n-Sustainable-Use-License")
    );
    assert_eq!(
        m.license_url.as_deref(),
        Some("https://docs.n8n.io/sustainable-use-license/")
    );
    PluginPackage::parse(&compose("  license: LicenseRef-joplin"), &langs()).unwrap();
}

/// 比核心新的清单（`next_spec`）可能用了核心不认识的字段：先报规范版本，类别是 unsupported（要升级的是核心）。
#[test]
fn newer_spec_is_reported_before_unknown_fields() {
    let newer = next_spec();
    let yaml = without_spec(&format!(
        "  spec: \"{newer}\"\n  shortcuts: []\n  webui: {{service: web, port: 80, not_yet: true}}"
    ));
    let err = PluginPackage::parse(&yaml, &langs()).unwrap_err();
    assert!(err.to_string().contains(&format!("规范 {newer}")), "{err}");
    assert!(err.to_string().contains("先升级核心"), "{err}");
    assert_eq!(InstallError::from(err).kind(), "unsupported");
    let reject = |yaml: String| {
        PluginPackage::parse(&yaml, &langs())
            .unwrap_err()
            .to_string()
    };
    let version = super::SPEC_VERSION.to_string();
    assert!(reject(without_spec("  spec: 0.2")).contains(&message(
        "plugin-settings-store-info-spec-string",
        &[("version", &version)],
    )));
    assert!(reject(without_spec("  spec: \"v2\"")).contains(&message(
        "plugin-settings-store-info-spec-version",
        &[("text", "v2"), ("version", &version)],
    )));
}

/// 同一份清单写核心支持的版本时，未知字段照常按字段报（类别 invalid_input），不冒充“核心太旧”。
#[test]
fn an_unknown_field_under_the_supported_spec_is_invalid() {
    let current = super::SPEC_VERSION.to_string();
    let yaml = without_spec(&format!("  spec: \"{current}\"\n  shortcuts: []"));
    let err = PluginPackage::parse(&yaml, &langs()).unwrap_err();
    assert!(err.to_string().contains("shortcuts"), "{err}");
    assert_eq!(InstallError::from(err).kind(), "invalid_input");
}

#[test]
fn a_manifest_without_spec_is_rejected() {
    let err = PluginPackage::parse(&without_spec(""), &langs()).unwrap_err();
    assert!(
        err.to_string().contains(&message(
            "plugin-settings-store-info-spec-missing",
            &[("version", &super::SPEC_VERSION.to_string())],
        )),
        "{err}"
    );
}

/// 许可证只认 SPDX 表达式或 `LicenseRef-`，`license_url` 要 HTTPS、要伴着 `license` 写。
#[test]
fn license_and_license_url_are_checked() {
    let cases = [
        ("  license: Apache 2", "SPDX".to_owned()),
        (
            "  license_url: https://example.com/eula",
            message("plugin-settings-store-info-license-url-alone", &[]),
        ),
        (
            "  license: MIT\n  license_url: http://example.com/eula",
            message(
                "plugin-settings-store-info-https-url",
                &[
                    ("field", "license_url"),
                    ("value", "http://example.com/eula"),
                ],
            ),
        ),
    ];
    for (extra, expected) in cases {
        let err = parse_err(extra);
        assert!(err.contains(&expected), "{extra}：{err}");
    }
}

#[test]
fn store_fields_are_checked() {
    let long = "字".repeat(4001);
    let notes = "字".repeat(2001);
    let tags9 = (0..9)
        .map(|i| format!("t{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let shots7 = (0..7)
        .map(|i| format!("s{i}.png"))
        .collect::<Vec<_>>()
        .join(", ");
    let cases: Vec<(String, String)> = vec![
        ("  categories: [ai]".to_owned(), "categories".to_owned()),
        (
            "  homepage: http://example.com".to_owned(),
            "https://".to_owned(),
        ),
        (
            "  author: {name: x, url: http://example.com}".to_owned(),
            "https://".to_owned(),
        ),
        (
            "  author: {name: \"  \"}".to_owned(),
            "author.name".to_owned(),
        ),
        (
            format!("  tags: {{zh: [{tags9}]}}"),
            message("plugin-settings-store-info-tags-count", &[("max", "8")]),
        ),
        (
            format!("  tags: {{en: [{}]}}", "a".repeat(25)),
            message(
                "plugin-settings-store-info-tag-length",
                &[("tag", &"a".repeat(25)), ("max", "24")],
            ),
        ),
        (
            "  tags: {en: [Photo, photo]}".to_owned(),
            message(
                "plugin-settings-store-info-tag-duplicate",
                &[("tag", "photo")],
            ),
        ),
        (
            "  about: {zh: \"\", en: x}".to_owned(),
            "至少要写一种语言".to_owned(),
        ),
        (
            format!("  about: {{zh: {long}, en: x}}"),
            message(
                "plugin-settings-store-info-field-length",
                &[("field", "about"), ("lang", "zh"), ("max", "4000")],
            ),
        ),
        (
            format!("  release_notes: {{zh: {notes}, en: x}}"),
            message(
                "plugin-settings-store-info-field-length",
                &[("field", "release_notes"), ("lang", "zh"), ("max", "2000")],
            ),
        ),
        (
            "  screenshots: []".to_owned(),
            message(
                "plugin-settings-store-info-screenshots-count",
                &[("max", "6")],
            ),
        ),
        (
            format!("  screenshots: [{shots7}]"),
            message(
                "plugin-settings-store-info-screenshots-count",
                &[("max", "6")],
            ),
        ),
        (
            "  screenshots: [a.gif]".to_owned(),
            message(
                "plugin-settings-store-info-screenshot-format",
                &[("raw", "a.gif")],
            ),
        ),
        ("  screenshots: [../a.png]".to_owned(), "..".to_owned()),
        (
            "  screenshots: [a.png, ./a.png]".to_owned(),
            message(
                "plugin-settings-store-info-screenshot-duplicate",
                &[("raw", "./a.png")],
            ),
        ),
    ];
    for (extra, expected) in cases {
        let err = parse_err(&extra);
        assert!(err.contains(&expected), "{extra}：{err}");
    }
}

#[test]
fn icon_must_be_a_png_or_svg_in_the_package() {
    for icon in ["https://example.com/icon.png", "icon.gif", "/etc/icon.png"] {
        let err = parse_err(&format!("  icon: {icon}"));
        assert!(err.contains("icon"), "{icon}：{err}");
    }
    let err = parse_err("  icon: icon.svg");
    assert!(err.contains("does not exist"), "{err}");
    let files = PackageFiles::from([("icon.svg".to_owned(), b"<svg/>".to_vec())]);
    PluginPackage::with_files(&compose("  icon: icon.svg"), files, &langs()).unwrap();
}

#[test]
fn package_size_is_limited() {
    let count = MAX_PACKAGE_BYTES / MAX_FILE_BYTES + 1;
    let mut yaml = compose("") + "configs:\n";
    let mut files = PackageFiles::new();
    for i in 0..count {
        yaml += format!("  c{i}:\n    file: c{i}.bin\n").as_str();
        files.insert(format!("c{i}.bin"), vec![b'x'; MAX_FILE_BYTES as usize]);
    }
    let err = PluginPackage::with_files(&yaml, files, &langs()).unwrap_err();
    assert!(matches!(err, PackageError::TooBig(_)), "{err}");
}

#[test]
fn notices_travel_with_the_package_and_stay_text() {
    let yaml = compose("  notices: [NOTICE, licenses/MIT.txt]");
    let err = PluginPackage::with_files(&yaml, PackageFiles::new(), &langs()).unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");

    let mit = ("licenses/MIT.txt".to_owned(), b"MIT".to_vec());
    let files = PackageFiles::from([("NOTICE".to_owned(), b"source".to_vec()), mit.clone()]);
    let package = PluginPackage::with_files(&yaml, files.clone(), &langs()).unwrap();
    assert_eq!(package.files(), &files);
    let entry = IndexEntry::of(&package, "github:acme/hello", &"c".repeat(40)).unwrap();
    assert!(entry.files.contains_key("NOTICE"), "{entry:?}");

    let binary = PackageFiles::from([("NOTICE".to_owned(), vec![0x89, b'P', 0xff]), mit]);
    let err = PluginPackage::with_files(&yaml, binary, &langs()).unwrap_err();
    let not_utf8 = crate::plugin::package_files::FileError::NotText.to_string();
    let expected = crate::l10n::text(
        &langs(),
        "plugin-install-pkg-file",
        &[
            ("field", "notices".into()),
            ("path", "NOTICE".into()),
            ("error", not_utf8.into()),
        ],
    );
    assert_eq!(err.to_string(), expected, "{err}");
}
