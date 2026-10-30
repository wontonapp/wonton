use super::super::InstallError;
use super::super::package::PluginPackage;
use super::super::package_files::PackageFiles;
use super::*;

fn compose(extra: &str) -> String {
    format!(
        "services:\n  web:\n    image: nginx\nx-wonton:\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {{zh: 测试, en: Test}}\n  description: {{zh: 测试, en: Test}}\n  permissions: []\n{extra}"
    )
}

fn parse_err(extra: &str) -> String {
    match PluginPackage::parse(&compose(extra)) {
        Ok(_) => panic!("应被拒绝：{extra}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn store_info_of_spec_0_2_parses() {
    let yaml = compose(
        "  spec: \"0.2\"\n  category: photos\n  author: {name: Example, url: https://example.com}\n  \
         homepage: https://immich.app\n  license: AGPL-3.0-only\n  tags: [照片, immich]\n  \
         about: {zh: 第一段, en: First}\n  release_notes: {zh: 修了问题, en: Fixes}\n  \
         screenshots: [shots/1.png, shots/2.jpg]",
    );
    let m = PluginPackage::parse(&yaml).unwrap().manifest;
    assert_eq!(m.category, Some(Category::Photos));
    assert_eq!(m.author.unwrap().name, "Example");
    assert_eq!(m.tags, ["照片", "immich"]);
    assert_eq!(m.screenshots.unwrap().len(), 2);
    let dual = compose("  spec: \"0.2\"\n  license: MIT OR Apache-2.0");
    PluginPackage::parse(&dual).unwrap();
}

/// 比核心新的清单可能用了核心不认识的字段：先报规范版本，类别是 unsupported（要升级的是核心）。
#[test]
fn newer_spec_is_reported_before_unknown_fields() {
    let yaml = compose("  spec: \"0.3\"\n  shortcuts: []");
    let err = PluginPackage::parse(&yaml).unwrap_err();
    assert!(err.to_string().contains("规范 0.3"), "{err}");
    assert!(err.to_string().contains("先升级核心"), "{err}");
    assert_eq!(InstallError::from(err).kind(), "unsupported");
    assert!(parse_err("  spec: 0.2").contains("加引号"));
    assert!(parse_err("  spec: \"v2\"").contains("不是规范版本"));
}

#[test]
fn spec_0_2_fields_need_the_spec_field() {
    let err = parse_err("  license: MIT");
    assert!(err.contains("spec: \"0.2\""), "{err}");
    // 0.1 就有的字段不需要。
    PluginPackage::parse(&compose("  category: tools")).unwrap();
}

#[test]
fn store_fields_are_checked() {
    let spec = "  spec: \"0.2\"\n";
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
        ("  category: gaming".to_owned(), "gaming"),
        ("  category: system".to_owned(), "系统插件"),
        ("  license: Apache 2".to_owned(), "SPDX"),
        ("  homepage: http://example.com".to_owned(), "https://"),
        (
            "  author: {name: x, url: http://example.com}".to_owned(),
            "https://",
        ),
        ("  author: {name: \"  \"}".to_owned(), "author.name"),
        (format!("  tags: [{tags9}]"), "最多 8"),
        (format!("  tags: [{}]", "a".repeat(25)), "1 到 24"),
        ("  tags: [Photo, photo]".to_owned(), "重复"),
        ("  about: {zh: 只有中文}".to_owned(), "zh 和 en"),
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
        let err = parse_err(&format!("{spec}{extra}"));
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
