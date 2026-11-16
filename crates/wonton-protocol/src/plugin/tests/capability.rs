//! 按能力依赖（ADR-0053）的线格式契约测试：缺失能力、轻提示、提供方设置、卸载预览、升级拦截。

use serde_json::json;

use super::*;

/// 契约：缺的 `requires`/`wants` 能力的 JSON 写法，旧核心不写这些字段也读得懂（ADR-0053）。
#[test]
fn capability_dependencies_keep_their_wire_format() {
    assert_eq!(
        serde_json::to_value(&preview().missing_requires[0]).unwrap(),
        json!({
            "capability": "jellyfin",
            "third_party": false,
            "providers": [{
                "id": "app.jellyfin",
                "title": "Jellyfin",
                "source": "github:example/wonton-jellyfin",
                "high_privilege": false,
            }],
        })
    );
    assert_eq!(
        serde_json::to_value(&preview().missing_wants[0]).unwrap(),
        json!({"capability": "x-acme.ledger", "third_party": true, "providers": []})
    );
    let old: InstallPreview = serde_json::from_value(json!({
        "digest": "sha256:00",
        "id": "app.samba",
        "title": "Samba",
        "summary": "",
        "version": "1.0.0",
        "kind": "app",
        "trust": "community",
        "permissions": [],
        "notices": [],
        "always_allowed": [],
    }))
    .unwrap();
    assert!(old.missing_requires.is_empty());
    assert!(old.missing_wants.is_empty());
    assert!(
        old.wanting_plugins.is_empty(),
        "old core omits this field; wanting plugins default to empty"
    );
}

/// 契约：装提供方时列出已装、`wants` 这个能力的插件，主人勾选的放进安装请求（ADR-0053 的
/// “高权限能力”，提供方后装不自动接上）。
#[test]
fn wanting_plugins_keep_their_wire_format() {
    let preview = InstallPreview {
        wanting_plugins: vec![CapabilityProviderRef {
            id: "app.seerr".into(),
            title: "Seerr".into(),
            high_privilege: false,
        }],
        ..preview()
    };
    assert_eq!(
        serde_json::to_value(&preview).unwrap()["wanting_plugins"],
        json!([{"id": "app.seerr", "title": "Seerr", "high_privilege": false}])
    );
    assert_eq!(round_trip(&preview), preview);

    let request = InstallRequest {
        connect_plugins: vec!["app.seerr".into()],
        ..request_without_extras()
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap()["connect_plugins"],
        json!(["app.seerr"])
    );
    assert_eq!(round_trip(&request), request);
    // 旧 App 不带勾选：按空算，一个都不接。
    let old: InstallRequest = serde_json::from_value(json!({
        "source": "github:example/wonton-jellyfin@v1.0.0",
        "digest": "sha256:00",
        "grant": [],
    }))
    .unwrap();
    assert!(old.connect_plugins.is_empty());
    assert!(old.install_together.is_empty());
}

fn request_without_extras() -> InstallRequest {
    InstallRequest {
        source: "github:example/wonton-jellyfin@v1.0.0".into(),
        digest: format!("sha256:{}", "cd".repeat(32)),
        plan: "provider-plan".into(),
        audience: None,
        scope: None,
        grant: vec![],
        devices: vec![],
        lan_webui: false,
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: Default::default(),
        install_together: vec![],
        connect_plugins: vec![],
        remote_oauth: None,
    }
}

/// 契约：依赖能力的轻提示类别是 snake_case，带提供方（ID 和标题）或能力名；不认识的类别落到 `Other`（ADR-0053）。
#[test]
fn capability_hints_keep_their_wire_format() {
    let stopped = PluginHint {
        category: PluginHintCategory::DependencyStopped,
        detail: None,
        provider: Some(CapabilityProviderRef {
            id: "app.model.gateway".into(),
            title: "Model gateway".into(),
            high_privilege: false,
        }),
        capability: None,
    };
    assert_eq!(
        serde_json::to_value(&stopped).unwrap(),
        json!({
            "category": "dependency_stopped",
            "detail": null,
            "provider": {"id": "app.model.gateway", "title": "Model gateway", "high_privilege": false},
            "capability": null,
        })
    );
    assert_eq!(round_trip(&stopped), stopped);
    let waiting = PluginHint {
        category: PluginHintCategory::ProviderNotConnected,
        detail: None,
        provider: Some(CapabilityProviderRef {
            id: "app.jellyfin".into(),
            title: "Jellyfin".into(),
            high_privilege: false,
        }),
        capability: Some("jellyfin".into()),
    };
    assert_eq!(
        serde_json::to_value(&waiting).unwrap()["category"],
        json!("provider_not_connected")
    );
    assert_eq!(round_trip(&waiting), waiting);
    let wants = PluginHint {
        category: PluginHintCategory::WantsUnsatisfied,
        detail: None,
        provider: None,
        capability: Some("jellyfin".into()),
    };
    assert_eq!(
        serde_json::to_value(&wants).unwrap()["category"],
        json!("wants_unsatisfied")
    );
    assert_eq!(round_trip(&wants), wants);
    assert_eq!(round_trip(&detail().hints[0]), detail().hints[0]);
    assert_eq!(
        serde_json::from_value::<PluginHintCategory>(json!("serving_from_a_mirror")).unwrap(),
        PluginHintCategory::Other
    );
}

/// 契约：提供方设置的读写和卸载预览的 JSON 写法（ADR-0053）。
#[test]
fn provider_settings_and_uninstall_keep_their_wire_format() {
    let settings = CapabilityProviderSettings {
        capabilities: vec![CapabilitySetting {
            capability: "jellyfin".into(),
            third_party: false,
            providers: vec![CapabilityProviderRef {
                id: "app.jellyfin".into(),
                title: "Jellyfin".into(),
                high_privilege: false,
            }],
            default_provider: "app.jellyfin".into(),
        }],
    };
    assert_eq!(round_trip(&settings), settings);
    assert_eq!(
        serde_json::to_value(&settings.capabilities[0]).unwrap()["default_provider"],
        json!("app.jellyfin")
    );

    let default = CapabilityDefaultChoice {
        provider: "app.jellyfin-2".into(),
    };
    assert_eq!(json!(default), json!({"provider": "app.jellyfin-2"}));

    let consumer = ConsumerCapabilityChoice {
        capability: "jellyfin".into(),
        provider: None,
    };
    assert_eq!(
        json!(consumer),
        json!({"capability": "jellyfin", "provider": null}),
        "empty means follow the global default"
    );

    // 卸载被拦的错误：kind 在核心的 HTTP 层写，这里带依赖方的 ID 和标题。
    let in_use = InUseError {
        plugins: vec![
            CapabilityProviderRef {
                id: "app.seerr".into(),
                title: "Seerr".into(),
                high_privilege: false,
            },
            CapabilityProviderRef {
                id: "app.prowlarr".into(),
                title: "Prowlarr".into(),
                high_privilege: false,
            },
        ],
    };
    assert_eq!(
        json!(in_use),
        json!({"plugins": [
            {"id": "app.seerr", "title": "Seerr", "high_privilege": false},
            {"id": "app.prowlarr", "title": "Prowlarr", "high_privilege": false},
        ]})
    );

    let uninstall = UninstallPreview {
        id: "app.seerr".into(),
        unused_providers: vec![CapabilityProviderRef {
            id: "app.jellyfin".into(),
            title: "Jellyfin".into(),
            high_privilege: false,
        }],
    };
    assert_eq!(round_trip(&uninstall), uninstall);
    let request = UninstallRequest {
        providers: vec!["app.jellyfin".into()],
    };
    assert_eq!(json!(request), json!({"providers": ["app.jellyfin"]}));
}

/// 契约：升级预览“装不了”的类别是 snake_case，不认识的类别落到 `Other`（ADR-0053）。
#[test]
fn blocked_categories_fall_back_to_other() {
    assert_eq!(
        serde_json::to_value(BlockedCategory::CapabilityDropped).unwrap(),
        json!("capability_dropped")
    );
    assert_eq!(
        serde_json::to_value(BlockedCategory::DeviceUnavailable).unwrap(),
        json!("device_unavailable")
    );
    assert_eq!(
        serde_json::from_value::<BlockedCategory>(json!("provider_migrating")).unwrap(),
        BlockedCategory::Other
    );
    let preview = InstallPreview {
        blocked_category: Some(BlockedCategory::CapabilityDropped),
        ..preview()
    };
    assert_eq!(round_trip(&preview), preview);
}

/// 旧核心给的设备项没有类别、系统盘标记和选中状态：按不认识的类、没标、没选读。
#[test]
fn devices_from_older_cores_default_the_new_fields() {
    let device: PluginDevice = serde_json::from_value(json!({
        "id": "usb-1", "name": "Zigbee 棒", "target": "/dev/zigbee",
        "service": "zigbee2mqtt", "optional": false
    }))
    .unwrap();
    assert_eq!(device.kind, PluginDeviceKind::Other);
    assert!(!device.system && !device.chosen);
}

#[test]
fn granted_devices_preserve_unknown_kinds_and_missing_devices() {
    let devices: Vec<PluginGrantedDevice> = serde_json::from_value(json!([
        {"path": "/dev/disk/by-id/ata-1", "name": "WDC", "kind": "disk", "system": true, "available": true},
        {"path": "/dev/bus/usb", "kind": "usb", "available": false},
        {"path": "/dev/rfkill", "name": "Bluetooth", "kind": "bluetooth", "available": true}
    ]))
    .unwrap();
    assert_eq!(devices[0].kind, PluginDeviceKind::Disk);
    assert!(devices[0].system && devices[0].available);
    assert_eq!(devices[1].kind, PluginDeviceKind::Usb);
    assert_eq!(devices[1].name, None);
    assert!(!devices[1].available);
    assert_eq!(devices[2].kind, PluginDeviceKind::Other);
    assert_eq!(devices[2].path, "/dev/rfkill");
    assert_eq!(round_trip(&devices), devices);

    let mut old = serde_json::to_value(detail()).unwrap();
    old.as_object_mut().unwrap().remove("devices");
    let old: PluginDetail = serde_json::from_value(old).unwrap();
    assert!(old.devices.is_empty());
}
