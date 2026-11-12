use super::super::InstallError;
use super::super::package::PluginPackage;
use super::super::package_files::PackageFiles;
use super::super::storefront::index::IndexEntry;

fn without_spec(extra: &str) -> String {
    format!(
        "services:\n  web:\n    image: nginx\nx-wonton:\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {{zh: 测试, en: Test}}\n  description: {{zh: 测试, en: Test}}\n  permissions: []\n{extra}"
    )
}

fn compose(extra: &str) -> String {
    without_spec(&format!("  spec: '0.5'\n{extra}"))
}

fn parse_err(extra: &str) -> String {
    match PluginPackage::parse(&compose(extra)) {
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
    let m = PluginPackage::parse(&yaml).unwrap().manifest;
    assert_eq!(m.author.unwrap().name, "Example");
    assert_eq!(m.tags["en"], ["Photos", "AI"]);
    assert_eq!(m.screenshots.unwrap().len(), 2);
    let dual = compose("  license: MIT OR Apache-2.0");
    PluginPackage::parse(&dual).unwrap();
}

/// 比核心新的清单可能用了核心不认识的字段：先报规范版本，类别是 unsupported（要升级的是核心）。
#[test]
fn newer_spec_is_reported_before_unknown_fields() {
    let yaml = without_spec("  spec: \"0.7\"\n  shortcuts: []");
    let err = PluginPackage::parse(&yaml).unwrap_err();
    assert!(err.to_string().contains("规范 0.7"), "{err}");
    assert!(err.to_string().contains("先升级核心"), "{err}");
    assert_eq!(InstallError::from(err).kind(), "unsupported");
    let reject = |yaml: String| PluginPackage::parse(&yaml).unwrap_err().to_string();
    assert!(reject(without_spec("  spec: 0.2")).contains("加引号"));
    assert!(reject(without_spec("  spec: \"v2\"")).contains("不是规范版本"));
}

#[test]
fn a_manifest_without_spec_is_rejected() {
    let err = PluginPackage::parse(&without_spec("")).unwrap_err();
    assert!(err.to_string().contains("要写 spec"), "{err}");
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
    let cases = [
        ("  categories: [ai]".to_owned(), "categories"),
        ("  license: Apache 2".to_owned(), "SPDX"),
        ("  homepage: http://example.com".to_owned(), "https://"),
        (
            "  author: {name: x, url: http://example.com}".to_owned(),
            "https://",
        ),
        ("  author: {name: \"  \"}".to_owned(), "author.name"),
        (format!("  tags: {{zh: [{tags9}]}}"), "最多 8"),
        (format!("  tags: {{en: [{}]}}", "a".repeat(25)), "1 到 24"),
        ("  tags: {en: [Photo, photo]}".to_owned(), "重复"),
        ("  about: {zh: \"\", en: x}".to_owned(), "至少要写一种语言"),
        (format!("  about: {{zh: {long}, en: x}}"), "超过 4000"),
        (
            format!("  release_notes: {{zh: {notes}, en: x}}"),
            "超过 2000",
        ),
        ("  screenshots: []".to_owned(), "1 到 6"),
        (format!("  screenshots: [{shots7}]"), "1 到 6"),
        ("  screenshots: [a.gif]".to_owned(), "PNG 或 JPEG"),
        ("  screenshots: [../a.png]".to_owned(), ".."),
        ("  screenshots: [a.png, ./a.png]".to_owned(), "重复"),
    ];
    for (extra, expected) in cases {
        let err = parse_err(&extra);
        assert!(err.contains(expected), "{extra}：{err}");
    }
}

#[test]
fn icon_must_be_a_png_or_svg_in_the_package() {
    for icon in ["https://example.com/icon.png", "icon.gif", "/etc/icon.png"] {
        let err = parse_err(&format!("  icon: {icon}"));
        assert!(err.contains("icon"), "{icon}：{err}");
    }
    let err = parse_err("  icon: icon.svg");
    assert!(err.contains("不存在"), "{err}");
    let files = PackageFiles::from([("icon.svg".to_owned(), b"<svg/>".to_vec())]);
    PluginPackage::with_files(&compose("  icon: icon.svg"), files).unwrap();
}

#[test]
fn package_size_is_limited() {
    let yaml = compose("") + "configs:\n  a:\n    file: a.bin\n  b:\n    file: b.bin\n";
    let mib = vec![b'x'; 1 << 20];
    let files = PackageFiles::from([("a.bin".to_owned(), mib.clone()), ("b.bin".to_owned(), mib)]);
    let err = PluginPackage::with_files(&yaml, files).unwrap_err();
    assert!(
        err.to_string().contains("一共 2049 KiB，超过 2048 KiB"),
        "{err}"
    );
}

#[test]
fn notices_travel_with_the_package_and_stay_text() {
    let yaml = compose("  notices: [NOTICE, licenses/MIT.txt]");
    let err = PluginPackage::with_files(&yaml, PackageFiles::new()).unwrap_err();
    assert!(err.to_string().contains("不存在"), "{err}");

    let mit = ("licenses/MIT.txt".to_owned(), b"MIT".to_vec());
    let files = PackageFiles::from([("NOTICE".to_owned(), b"source".to_vec()), mit.clone()]);
    let package = PluginPackage::with_files(&yaml, files.clone()).unwrap();
    assert_eq!(package.files(), &files);
    let entry = IndexEntry::of(&package, "github:acme/hello", &"c".repeat(40)).unwrap();
    assert!(entry.files.contains_key("NOTICE"), "{entry:?}");

    let binary = PackageFiles::from([("NOTICE".to_owned(), vec![0x89, b'P', 0xff]), mit]);
    let err = PluginPackage::with_files(&yaml, binary).unwrap_err();
    assert!(err.to_string().contains("不是 UTF-8 文本"), "{err}");
}
