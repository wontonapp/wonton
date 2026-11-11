use super::super::manifest::ManifestTab;
use super::super::package::{PackageError, PluginPackage};
use super::super::package_files::PackageFiles;
use super::package;

const SERVICES: &str = "  web:\n    image: nginx";
const WEBUI: &str = "  webui: {service: web, port: 8080}";
const ICON: &str = "tabs/feed.svg";

fn tab(extra: &str) -> String {
    format!(
        "{WEBUI}\n  tabs:\n    - id: feed\n      title: {{zh: 动态, en: Feed}}\n      icon: {ICON}\n      path: /feed\n      layout: inset\n      display: standard\n{extra}  permissions: []"
    )
}

fn parse(extra: &str) -> Result<PluginPackage, PackageError> {
    PluginPackage::parse(&package(SERVICES, &tab(extra)), &super::langs())
}

fn error(extra: &str) -> String {
    parse(extra).unwrap_err().to_string()
}

#[test]
fn valid_tabs_keep_all_declared_fields_and_defaults() {
    let source = tab("      icon_selected: tabs/feed-fill.svg\n")
        .replace("layout: inset", "layout: edge-to-edge")
        .replace("display: standard", "display: fullscreen");
    let files = PackageFiles::from([
        (ICON.to_owned(), b"svg".to_vec()),
        ("tabs/feed-fill.svg".to_owned(), b"svg".to_vec()),
    ]);
    let yaml = package(SERVICES, &source);
    let package = PluginPackage::with_files(&yaml, files, &super::langs()).unwrap();
    let [tab] = package.manifest.tabs.as_deref().unwrap() else {
        panic!("应有一项 tabs")
    };
    assert_eq!(tab.id, "feed");
    assert_eq!(tab.title["zh"], "动态");
    assert_eq!(tab.title["en"], "Feed");
    assert_eq!(tab.icon.as_deref(), Some(ICON));
    assert_eq!(tab.icon_selected.as_deref(), Some("tabs/feed-fill.svg"));
    assert_eq!(tab.path, "/feed");
    assert_eq!(tab.layout, "edge-to-edge");
    assert_eq!(tab.display, "fullscreen");
}

#[test]
fn tab_validation_reports_each_field_rule() {
    let cases = [
        ("    - id: feed", "    - id: Bad_feed", "tabs[0].id"),
        (
            "      title: {zh: 动态, en: Feed}",
            "      title: {en: Feed}",
            "tabs[0].title",
        ),
        (
            "      icon: tabs/feed.svg",
            "      icon: null",
            "tabs[0].icon",
        ),
        ("      path: /feed", "      path: feed", "tabs[0].path"),
        (
            "      layout: inset",
            "      layout: sidebar",
            "tabs[0].layout",
        ),
        (
            "      display: standard",
            "      display: picture-in-picture",
            "tabs[0].display",
        ),
    ];
    for (original, replacement, field) in cases {
        let source = tab("").replace(original, replacement);
        let err = PluginPackage::parse(&package(SERVICES, &source), &super::langs())
            .unwrap_err()
            .to_string();
        assert!(err.contains(field), "{field}: {err}");
    }

    for (path, expected) in [
        ("//evil.example/x", "//"),
        ("/\\evil.example/x", "backslash"),
    ] {
        let source = tab("").replace("      path: /feed", &format!("      path: '{path}'"));
        let err = PluginPackage::parse(&package(SERVICES, &source), &super::langs())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("tabs[0].path") && err.contains(expected),
            "{err}"
        );
    }

    let source = tab("").replace("      path: /feed", "      path: \"/feed\\u0001\"");
    let err = PluginPackage::parse(&package(SERVICES, &source), &super::langs())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("tabs[0].path") && err.contains("control characters"),
        "{err}"
    );

    let duplicate = "    - id: feed\n      title: {zh: 第二, en: Second}\n      icon: tabs/second.svg\n      path: /second\n";
    assert!(error(duplicate).contains("tabs[1].id"));

    let too_many = (0..5)
        .map(|i| {
            format!(
                "    - id: page-{i}\n      title: {{zh: 页面, en: Page}}\n      icon: tabs/{i}.svg\n      path: /page-{i}\n"
            )
        })
        .collect::<String>();
    assert!(error(&too_many).contains("tabs allows at most 4 entries"));

    let no_webui = package(
        SERVICES,
        "  tabs:\n    - id: feed\n      title: {zh: 动态, en: Feed}\n      icon: tabs/feed.svg\n      path: /feed\n  permissions: []",
    );
    assert!(
        PluginPackage::parse(&no_webui, &super::langs())
            .unwrap_err()
            .to_string()
            .contains("tabs is declared but webui is not")
    );

    let bad_display = package(
        SERVICES,
        "  webui: {service: web, port: 8080, display: picture-in-picture}\n  permissions: []",
    );
    assert!(
        PluginPackage::parse(&bad_display, &super::langs())
            .unwrap_err()
            .to_string()
            .contains("webui.display")
    );
}

#[test]
fn entry_paths_may_carry_a_query_and_must_be_percent_encoded() {
    // noVNC 这类入口路径：query 原样保留，核心不再编码。
    let novnc = tab("").replace(
        "      path: /feed",
        "      path: /vnc.html?autoconnect=1&resize=remote",
    );
    let novnc_package = PluginPackage::with_files(
        &package(SERVICES, &novnc),
        PackageFiles::from([(ICON.to_owned(), b"svg".to_vec())]),
        &super::langs(),
    )
    .unwrap();
    let [declared] = novnc_package.manifest.tabs.as_deref().unwrap() else {
        panic!("应有一项 tabs")
    };
    assert_eq!(declared.path, "/vnc.html?autoconnect=1&resize=remote");

    let with_query = tab("").replace(
        WEBUI,
        "  webui: {service: web, port: 8080, path: '/vnc.html?autoconnect=1&resize=remote'}",
    );
    let manifest = PluginPackage::with_files(
        &package(SERVICES, &with_query),
        PackageFiles::from([(ICON.to_owned(), b"svg".to_vec())]),
        &super::langs(),
    )
    .unwrap()
    .manifest;
    assert_eq!(
        manifest.webui.unwrap().path,
        "/vnc.html?autoconnect=1&resize=remote"
    );

    // 同一条规则管 webui.path 和 tabs[].path：#、非 ASCII 的错误带正确的编码值，
    // 空白和不完整的 % 只断言字段名，不把文案钉进测试。
    for (bad, encoded) in [("/feed#frag", "%23"), ("/中文", "%E4%B8%AD")] {
        let source = tab("").replace("      path: /feed", &format!("      path: '{bad}'"));
        let err = PluginPackage::parse(&package(SERVICES, &source), &super::langs())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("tabs[0].path") && err.contains(encoded),
            "{err}"
        );

        let source = tab("").replace(
            WEBUI,
            &format!("  webui: {{service: web, port: 8080, path: '{bad}'}}"),
        );
        let err = PluginPackage::parse(&package(SERVICES, &source), &super::langs())
            .unwrap_err()
            .to_string();
        assert!(err.contains("webui.path") && err.contains(encoded), "{err}");
    }
    for bad in ["/fe ed", "/100%"] {
        let source = tab("").replace("      path: /feed", &format!("      path: '{bad}'"));
        let err = PluginPackage::parse(&package(SERVICES, &source), &super::langs())
            .unwrap_err()
            .to_string();
        assert!(err.contains("tabs[0].path"), "{err}");

        let source = tab("").replace(
            WEBUI,
            &format!("  webui: {{service: web, port: 8080, path: '{bad}'}}"),
        );
        let err = PluginPackage::parse(&package(SERVICES, &source), &super::langs())
            .unwrap_err()
            .to_string();
        assert!(err.contains("webui.path"), "{err}");
    }
}

#[test]
fn tab_icon_files_are_required_when_declared() {
    let source = tab("      icon_selected: tabs/missing.svg\n");
    let files = PackageFiles::from([(ICON.to_owned(), b"svg".to_vec())]);
    let err = PluginPackage::with_files(&package(SERVICES, &source), files, &super::langs())
        .unwrap_err()
        .to_string();
    assert!(err.contains("tabs[0].icon_selected"), "{err}");
    assert!(err.contains("does not exist"), "{err}");
}

#[test]
fn manifest_tab_type_is_usable_by_the_core() {
    let value = serde_json::json!({
        "id": "feed",
        "title": {"zh": "动态", "en": "Feed"},
        "icon": "tabs/feed.svg",
        "path": "/feed",
    });
    let tab: ManifestTab = serde_json::from_value(value).unwrap();
    assert_eq!(tab.layout, "inset");
    assert_eq!(tab.display, "standard");
}
