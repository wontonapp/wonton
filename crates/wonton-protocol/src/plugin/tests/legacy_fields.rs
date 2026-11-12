//! 旧核心缺字段、新核心多取值的线格式契约测试：缺的字段读成空值，不认识的取值落到 `Other`。

use serde_json::json;

use super::*;

#[test]
fn a_core_without_writers_lists_plugins_without_them() {
    let mut item = serde_json::to_value(installed()).unwrap();
    item.as_object_mut().unwrap().remove("written_by");
    let item: InstalledPlugin = serde_json::from_value(item).unwrap();
    assert_eq!(item.written_by, None);

    let mut shown = serde_json::to_value(detail()).unwrap();
    shown.as_object_mut().unwrap().remove("ask_every_update");
    shown.as_object_mut().unwrap().remove("hints");
    shown.as_object_mut().unwrap().remove("secret_inputs");
    let shown: PluginDetail = serde_json::from_value(shown).unwrap();
    assert_eq!(shown.ask_every_update, None);
    assert!(shown.hints.is_empty(), "旧核心没有这个字段，轻提示为空");
    assert!(
        shown.secret_inputs.is_empty(),
        "旧核心没有这个字段，没有要填的"
    );
}

#[test]
fn a_core_without_tabs_lists_plugins_without_pages() {
    let mut plugin = serde_json::to_value(installed()).unwrap();
    let fields = plugin.as_object_mut().unwrap();
    fields.remove("tabs");
    fields.remove("web_display");
    fields.remove("cross_origin_isolated");
    let plugin: InstalledPlugin = serde_json::from_value(plugin).unwrap();
    assert!(plugin.tabs.is_empty());
    assert_eq!(plugin.web_display, TabDisplay::Standard);
    assert!(!plugin.cross_origin_isolated);
}

/// 契约：旧核心没有 `install_progress`、`has_bot` 时读成空和 `false`；新核心的阶段旧 App 不认识的落到 `Other`。
#[test]
fn a_core_without_install_stages_and_a_new_stage_still_parse() {
    let mut plugin = serde_json::to_value(installed()).unwrap();
    plugin.as_object_mut().unwrap().remove("install_progress");
    let plugin: InstalledPlugin = serde_json::from_value(plugin).unwrap();
    assert!(plugin.install_progress.is_none());

    let mut shown = serde_json::to_value(preview()).unwrap();
    shown.as_object_mut().unwrap().remove("has_bot");
    let shown: InstallPreview = serde_json::from_value(shown).unwrap();
    assert!(!shown.has_bot);

    let progress: InstallProgress = serde_json::from_value(json!({
        "stage": "migrating",
        "percent": 40,
    }))
    .unwrap();
    assert_eq!(
        progress,
        InstallProgress {
            stage: InstallStage::Other,
            percent: Some(40),
        }
    );
}

/// 契约：旧核心没有 `widget_capabilities` 时读成空列表；新核心的级别旧 App 不认识的落到 `Other`。
#[test]
fn a_core_without_widget_capabilities_lists_none() {
    let mut shown = serde_json::to_value(detail()).unwrap();
    shown.as_object_mut().unwrap().remove("widget_capabilities");
    let shown: PluginDetail = serde_json::from_value(shown).unwrap();
    assert!(shown.widget_capabilities.is_empty());

    let mut confirmed = serde_json::to_value(preview()).unwrap();
    confirmed
        .as_object_mut()
        .unwrap()
        .remove("widget_capabilities");
    let confirmed: InstallPreview = serde_json::from_value(confirmed).unwrap();
    assert!(confirmed.widget_capabilities.is_empty());

    let unknown: WidgetCapability = serde_json::from_value(json!({
        "capability": "org.matrix.msc2762.receive.event:m.room.message",
        "level": "extreme",
    }))
    .unwrap();
    assert_eq!(unknown.level, WidgetCapabilityLevel::Other);
    assert!(!unknown.added);
}

/// 契约：旧核心没有 `matrix_widget` 时没有 widget；新核心的入口图标旧 App 不认识的落到 `Other`，缺的字段读成空值。
#[test]
fn a_core_without_matrix_widget_has_no_widget() {
    let mut item = serde_json::to_value(installed()).unwrap();
    item.as_object_mut().unwrap().remove("matrix_widget");
    let item: InstalledPlugin = serde_json::from_value(item).unwrap();
    assert_eq!(item.matrix_widget, None);

    let widget: PluginWidget = serde_json::from_value(json!({
        "url": "/room",
        "entry": "hologram",
    }))
    .unwrap();
    assert_eq!(widget.entry, WidgetEntry::Other);
    assert!(widget.capabilities.is_empty());
}
