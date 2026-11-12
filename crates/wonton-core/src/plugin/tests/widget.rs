//! 清单 `matrix_widget` 的契约：没有网页入口、和跨源隔离一起声明、通配、别的房间、不在入口下的
//! url 都装不上；能力的级别按事件类型算。

use wonton_protocol::WidgetCapabilityLevel;

use super::super::manifest::WidgetEntry;
use super::super::package::{PackageError, PluginPackage};

const SERVICES: &str = "  web:\n    image: nginx";
const WEBUI: &str = "  webui: {service: web, port: 8080}";

fn manifest(widget: &str) -> String {
    format!("{WEBUI}\n  matrix_widget:\n{widget}  permissions: []")
}

fn parse(widget: &str) -> Result<PluginPackage, PackageError> {
    PluginPackage::parse(
        &super::package(SERVICES, &manifest(widget)),
        &super::langs(),
    )
}

fn error(widget: &str) -> String {
    parse(widget).unwrap_err().to_string()
}

fn widget(capabilities: &[&str]) -> String {
    let mut yaml = String::from("    url: /room?widgetId=$matrix_widget_id\n");
    if capabilities.is_empty() {
        return yaml + "    capabilities: []\n";
    }
    yaml.push_str("    capabilities:\n");
    for capability in capabilities {
        // 空串在 YAML 里写成 `- ` 是 null 项，要加引号。
        yaml.push_str(&format!("      - {capability:?}\n"));
    }
    yaml
}

#[test]
fn a_widget_keeps_its_fields_and_capability_levels() {
    let yaml = widget(&[
        "org.matrix.msc2762.receive.state_event:org.matrix.msc3401.call.member",
        "org.matrix.msc2762.send.state_event:m.room.member#@user:example.org",
        "org.matrix.msc2762.send.event:org.matrix.rageshake_request",
        "org.matrix.msc3819.send.to_device:io.element.call.encryption_keys",
        "org.matrix.msc4157.send.delayed_event",
        "org.matrix.msc4515.rtc_transports",
    ])
    .replace("    url: /room", "    entry: voice_call\n    url: /room");
    let package = parse(&yaml).unwrap();
    let widget = package.manifest.matrix_widget.as_ref().unwrap();
    assert_eq!(widget.entry, WidgetEntry::VoiceCall);
    assert_eq!(widget.capabilities.len(), 6);
    let levels: Vec<WidgetCapabilityLevel> = widget
        .capabilities
        .iter()
        .map(|capability| super::super::manifest_widget::level(capability))
        .collect();
    assert_eq!(
        levels,
        vec![
            WidgetCapabilityLevel::Normal,
            WidgetCapabilityLevel::High,
            WidgetCapabilityLevel::Normal,
            WidgetCapabilityLevel::Normal,
            WidgetCapabilityLevel::Normal,
            WidgetCapabilityLevel::Normal,
        ]
    );
}

#[test]
fn an_unknown_entry_falls_back_to_other() {
    let yaml = widget(&[]).replace("    url: /room", "    entry: something\n    url: /room");
    let package = parse(&yaml).unwrap();
    assert_eq!(
        package.manifest.matrix_widget.as_ref().unwrap().entry,
        WidgetEntry::Other
    );
}

#[test]
fn a_widget_needs_a_web_entry() {
    let yaml = format!(
        "services:\n{SERVICES}\nx-wonton:\n  spec: '0.5'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {{zh: 测试, en: Test}}\n  description: {{zh: 测试, en: Test}}\n  matrix_widget:\n{}  permissions: []",
        widget(&[])
    );
    let error = PluginPackage::parse(&yaml, &super::langs())
        .unwrap_err()
        .to_string();
    assert!(error.contains("needs webui"), "{error}");
}

#[test]
fn a_widget_and_cross_origin_isolation_exclude_each_other() {
    let yaml = manifest(&widget(&[])).replace(
        "webui: {service: web, port: 8080}",
        "webui: {service: web, port: 8080, cross_origin_isolated: true}",
    );
    let error = PluginPackage::parse(&super::package(SERVICES, &yaml), &super::langs())
        .unwrap_err()
        .to_string();
    assert!(error.contains("cross_origin_isolated"), "{error}");
}

#[test]
fn wildcards_other_rooms_and_unknown_entries_are_refused() {
    for (capability, needle) in [
        ("org.matrix.msc2762.receive.event:*", "not an event type"),
        (
            "org.matrix.msc2762.receive.state_event:*",
            "not an event type",
        ),
        (
            "org.matrix.msc2762.send.state_event:m.room.member#*",
            "no wildcards",
        ),
        (
            "org.matrix.msc2762.send.state_event:m.room.member#",
            "nothing after #",
        ),
        (
            "org.matrix.msc2762.timeline:!room:example.org",
            "only gets the room",
        ),
        (
            "org.matrix.msc2762.send.to_device:io.element.call.encryption_keys",
            "not recognized",
        ),
        ("io.element.call.something", "not recognized"),
        ("", "empty entry"),
    ] {
        let error = error(&widget(&[capability]));
        assert!(error.contains(needle), "{capability}: {error}");
    }
}

#[test]
fn event_types_take_the_whole_matrix_character_set() {
    parse(&widget(&[
        "org.matrix.msc2762.receive.event:com.example.foo-bar",
        "org.matrix.msc2762.receive.event:com.example.a~b_C.2",
        "org.matrix.msc2762.send.event:org.matrix.MixedCase",
    ]))
    .unwrap();
}

#[test]
fn the_same_capability_twice_is_refused() {
    let capability = "org.matrix.msc4515.rtc_transports";
    let error = error(&widget(&[capability, capability]));
    assert!(error.contains("duplicated"), "{error}");
}

#[test]
fn the_url_stays_under_the_page() {
    for (url, needle) in [
        ("room", "must start with /"),
        ("//example.org/room", "must not start with //"),
        ("/room#x", "must not contain #"),
        ("/room x", "whitespace"),
    ] {
        let widget = format!("    url: {url}\n    capabilities: []\n");
        assert!(error(&widget).contains(needle), "{url}");
    }
    parse(&widget(&[])).unwrap();
}

#[test]
fn an_unknown_field_is_reported_with_its_path() {
    let widget = "    url: /room\n    capabilities: []\n    wat: 1\n";
    let error = error(widget);
    assert!(error.contains("x-wonton.matrix_widget.wat"), "{error}");
}

#[test]
fn only_new_capabilities_count_as_added() {
    let old = parse(&widget(&[
        "org.matrix.msc4515.rtc_transports",
        "org.matrix.msc4157.send.delayed_event",
    ]))
    .unwrap();
    let new = parse(&widget(&[
        "org.matrix.msc4515.rtc_transports",
        "org.matrix.msc2762.receive.event:m.room.message",
    ]))
    .unwrap();
    let added = super::super::manifest_widget::added(Some(&old.manifest), &new.manifest);
    assert_eq!(
        added,
        vec!["org.matrix.msc2762.receive.event:m.room.message"]
    );
    // 装着的清单读不出来时全算新增。
    let added = super::super::manifest_widget::added(None, &new.manifest);
    assert_eq!(added.len(), 2);
    // 新版本不再有 widget：没有新增。
    let plain = PluginPackage::parse(
        &super::package(SERVICES, &format!("{WEBUI}\n  permissions: []")),
        &super::langs(),
    )
    .unwrap();
    assert!(super::super::manifest_widget::added(Some(&old.manifest), &plain.manifest).is_empty());
}
