//! 修改 Bot 的输入密钥和可选权限时，重建保留 Matrix 身份。

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;

use super::{Bed, echo, granted, id};
use crate::plugin::gpu::fake_host::FakeHost;
use crate::plugin::manifest::Permission;
use crate::plugin::names::container_name;
use crate::plugin::store::OpenedBy;

fn assert_identity_kept(bed: &Bed, before: &BTreeMap<String, String>) {
    let after = bed.env();
    for key in ["HS", "USER", "TOKEN", "ALLOWED", "ROOM"] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert!(bed.token_valid(&after["TOKEN"]));
}

#[tokio::test]
async fn changing_an_input_secret_keeps_the_bot_identity() {
    let bed = Bed::start().await;
    let manager = Arc::new(bed.manager());
    let source = echo()
        .replace("spec: '0.5'", "spec: '0.8'")
        .replace(
            "ROOM: ${WONTON_MATRIX_HOME_ROOM}",
            "ROOM: ${WONTON_MATRIX_HOME_ROOM}\n      API_KEY: ${WONTON_SECRET_API_KEY}",
        )
        .replace(
            "  permissions: [matrix.bot]",
            "  permissions: [matrix.bot]\n  secrets:\n    api_key:\n      input:\n        title: {en: API key}\n        required: true",
        );
    manager
        .store
        .secrets()
        .save(&id(), &[("api_key".to_owned(), b"old-key".to_vec())].into())
        .unwrap();
    manager
        .install(&source, &granted(), super::super::langs())
        .await
        .unwrap();
    let before = bed.env();
    assert_eq!(before["API_KEY"], "old-key");

    manager
        .set_secret_input(&id(), "api_key", Some("new-key"), &super::super::langs())
        .await
        .unwrap();

    assert_eq!(bed.env()["API_KEY"], "new-key");
    assert_identity_kept(&bed, &before);
}

#[tokio::test]
async fn toggling_an_optional_permission_keeps_the_bot_identity() {
    let bed = Bed::start().await;
    let host = FakeHost::amd();
    let manager = Arc::new(bed.manager().with_host(host.paths()));
    let source = echo().replace(
        "  permissions: [matrix.bot]",
        "  permissions: [matrix.bot]\n  optional_permissions:\n    devices.gpu:\n      off_loss: {zh: 慢一些, en: Slower, zh-Hant: 慢一些, ja: 遅くなります}\n  gpu: {services: [bot]}",
    ).replace(
        "ROOM: ${WONTON_MATRIX_HOME_ROOM}",
        "ROOM: ${WONTON_MATRIX_HOME_ROOM}\n      GPU_ENABLED: ${WONTON_PERMISSION_DEVICES_GPU:-off}",
    );
    manager
        .install(&source, &granted(), super::super::langs())
        .await
        .unwrap();
    let before = bed.env();
    assert_eq!(before["GPU_ENABLED"], "off");

    for enabled in [true, false] {
        let detail = manager
            .set_plugin_permission(
                id(),
                Permission::DevicesGpu,
                enabled,
                OpenedBy::Owner,
                super::super::langs(),
            )
            .await
            .unwrap();
        let permission = detail
            .permissions
            .iter()
            .find(|line| line.permission == "devices.gpu")
            .unwrap();
        assert_eq!(permission.enabled, enabled);
        assert_eq!(bed.env()["GPU_ENABLED"], if enabled { "1" } else { "off" });
        let container = bed
            .docker
            .created(&container_name("app-echo", "bot"))
            .unwrap();
        assert_eq!(
            container["HostConfig"]["Devices"],
            if enabled {
                json!([
                    {"PathOnHost": "/dev/kfd", "PathInContainer": "/dev/kfd", "CgroupPermissions": "rwm"},
                    {"PathOnHost": "/dev/dri", "PathInContainer": "/dev/dri", "CgroupPermissions": "rwm"}
                ])
            } else {
                serde_json::Value::Null
            }
        );
        assert_identity_kept(&bed, &before);
    }
}
