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
    PluginPackage::parse(&package(SERVICES, &tab(extra)))
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
    let package = PluginPackage::with_files(&yaml, files).unwrap();
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
        let err = PluginPackage::parse(&package(SERVICES, &source))
            .unwrap_err()
            .to_string();
        assert!(err.contains(field), "{field}: {err}");
    }

    for (path, expected) in [("//evil.example/x", "//"), ("/\\evil.example/x", "反斜杠")] {
        let source = tab("").replace("      path: /feed", &format!("      path: '{path}'"));
        let err = PluginPackage::parse(&package(SERVICES, &source))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("tabs[0].path") && err.contains(expected),
            "{err}"
        );
    }

    let source = tab("").replace("      path: /feed", "      path: \"/feed\\u0001\"");
    let err = PluginPackage::parse(&package(SERVICES, &source))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("tabs[0].path") && err.contains("控制字符"),
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
    assert!(error(&too_many).contains("tabs 最多 4 项"));

    let no_webui = package(
        SERVICES,
        "  tabs:\n    - id: feed\n      title: {zh: 动态, en: Feed}\n      icon: tabs/feed.svg\n      path: /feed\n  permissions: []",
    );
    assert!(
        PluginPackage::parse(&no_webui)
            .unwrap_err()
            .to_string()
            .contains("tabs 声明了但没有声明 webui")
    );

    let bad_display = package(
        SERVICES,
        "  webui: {service: web, port: 8080, display: picture-in-picture}\n  permissions: []",
    );
    assert!(
        PluginPackage::parse(&bad_display)
            .unwrap_err()
            .to_string()
            .contains("webui.display")
    );
}

#[test]
fn tab_icon_files_are_required_when_declared() {
    let source = tab("      icon_selected: tabs/missing.svg\n");
    let files = PackageFiles::from([(ICON.to_owned(), b"svg".to_vec())]);
    let err = PluginPackage::with_files(&package(SERVICES, &source), files)
        .unwrap_err()
        .to_string();
    assert!(err.contains("tabs[0].icon_selected"), "{err}");
    assert!(err.contains("不存在"), "{err}");
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
