//! 标签怎么分配给插件：最后一段、去重、保留名、长度，以及安装记录里存下来的那个值。

use super::labels::{assign, ensure, validate_label};
use crate::plugin::manifest::PluginId;

fn taken(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn labels_take_the_last_id_segment() {
    assert_eq!(assign("app.immich.immich", &[]).unwrap(), "immich");
    assert_eq!(assign("io.github.alice.forgejo", &[]).unwrap(), "forgejo");
}

#[test]
fn labels_are_sanitized() {
    assert_eq!(assign("app.x.Forge-Jo_2", &[]).unwrap(), "forge-jo-2");
}

#[test]
fn duplicates_get_a_number() {
    assert_eq!(
        assign("app.immich.immich", &taken(&["immich"])).unwrap(),
        "immich-2"
    );
    assert_eq!(
        assign("app.immich.immich", &taken(&["immich", "immich-2"])).unwrap(),
        "immich-3"
    );
}

#[test]
fn lan_and_probe_are_reserved() {
    assert_eq!(assign("app.x.lan", &[]).unwrap(), "lan-2");
    assert_eq!(assign("app.x.probe", &[]).unwrap(), "probe-2");
    assert_eq!(assign("app.x.lan", &taken(&["lan-2"])).unwrap(), "lan-3");
    assert!(validate_label("lan").is_err());
    assert!(validate_label("probe").is_err());
}

#[test]
fn labels_stay_within_sixty_three_bytes() {
    let segment = "a".repeat(63);
    let id = format!("app.x.{segment}");
    let first = assign(&id, &[]).unwrap();
    assert_eq!(first.len(), 63);
    let second = assign(&id, &taken(&[&first])).unwrap();
    assert!(second.len() <= 63);
    assert!(second.ends_with("-2"), "{second}");
    validate_label(&second).unwrap();
}

#[test]
fn an_existing_label_survives_reinstall() {
    let id = PluginId::parse("app.immich.immich").unwrap();
    // 升级、重装时记录里已经有标签：照用，不因为自己占着这个名字就改名。
    assert_eq!(
        ensure(Some("immich"), &id, &taken(&["immich"])).unwrap(),
        "immich"
    );
    assert_eq!(ensure(None, &id, &[]).unwrap(), "immich");
}

/// 记录里没有这个字段（老记录）读成空，有就原样存回去。
#[test]
fn installed_record_carries_the_label() {
    let id = PluginId::parse("app.immich.immich").unwrap();
    let plain: crate::plugin::store::InstalledRecord = serde_json::from_str(
        r#"{"id":"app.immich.immich","version":"1","source":"","granted_permissions":[],"installed_at":0}"#,
    )
    .unwrap();
    let label = ensure(plain.entry_label.as_deref(), &id, &[]).unwrap();
    let mut record = plain;
    record.entry_label = Some(label);
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["entry_label"], "immich");
    let read: crate::plugin::store::InstalledRecord = serde_json::from_value(json).unwrap();
    assert_eq!(read.entry_label.as_deref(), Some("immich"));
}
