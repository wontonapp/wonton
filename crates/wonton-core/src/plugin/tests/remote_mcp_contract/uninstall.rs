use hyper::StatusCode;
use serde_json::json;

use super::fixture::{Bed, PROVIDER, REMOTE, SECRET, id, remote};
use crate::plugin::PluginPackage;
use crate::plugin::package::static_files::StaticFiles;
use crate::plugin::tests::langs;

#[tokio::test]
async fn remote_mcp_uninstall_preview_removes_consumer_before_selected_connector() {
    for status in [StatusCode::NO_CONTENT, StatusCode::INTERNAL_SERVER_ERROR] {
        let bed = Bed::new(true).await;
        bed.install("header").await.unwrap();
        *bed.fake.forget_status.lock().unwrap() = status;
        let preview = bed
            .manager
            .uninstall_preview(&id(REMOTE), &langs())
            .unwrap();
        let selected: Vec<_> = preview.unused_providers.into_iter().map(|p| p.id).collect();
        assert_eq!(selected, [PROVIDER]);

        let report = bed
            .manager
            .uninstall_detached(id(REMOTE), true, selected, false, langs())
            .await
            .unwrap();

        assert_eq!(
            *bed.fake.forgotten.lock().unwrap(),
            [format!("/_wonton/plugins/{REMOTE}")]
        );
        assert_eq!(report.warnings.is_empty(), status.is_success());
        assert!(bed.manager.record_for_test(&id(REMOTE)).is_none());
        assert!(bed.manager.record_for_test(&id(PROVIDER)).is_none());
        assert!(bed.manager.mcp_servers().is_empty());
        assert!(!bed.docker.containers().contains(&bed.provider_container()));
    }
}

#[tokio::test]
async fn remote_mcp_uninstall_batch_preserves_consumers_outside_the_preview() {
    const OTHER: &str = "com.example.other-remote-tools";
    let bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    let preview = bed
        .manager
        .uninstall_preview(&id(REMOTE), &langs())
        .unwrap();
    let selected: Vec<_> = preview.unused_providers.into_iter().map(|p| p.id).collect();
    assert_eq!(selected, [PROVIDER]);
    bed.manager
        .secrets_for_test(&id(OTHER), &[("token", SECRET)]);
    let other =
        PluginPackage::parse(&remote("header").source.replace(REMOTE, OTHER), &langs()).unwrap();
    bed.manager
        .install_package(other, &[], langs())
        .await
        .unwrap();

    let removing = [id(REMOTE), id(PROVIDER)];
    let error = bed
        .manager
        .check_uninstall_state(&id(PROVIDER), &removing, &langs())
        .unwrap_err();
    assert_eq!(error.kind(), "conflict");
    assert!(
        bed.manager
            .uninstall_preview(&id(REMOTE), &langs())
            .unwrap()
            .unused_providers
            .is_empty()
    );
    bed.manager
        .uninstall_detached(id(REMOTE), true, selected, false, langs())
        .await
        .unwrap_err();

    for plugin in [REMOTE, OTHER, PROVIDER] {
        assert!(bed.manager.record_for_test(&id(plugin)).is_some());
    }
    assert!(bed.fake.forgotten.lock().unwrap().is_empty());
    assert_eq!(bed.manager.mcp_servers().len(), 2);
    assert!(bed.docker.containers().contains(&bed.provider_container()));
}

#[tokio::test]
async fn remote_mcp_hosted_uninstall_preflight_also_handles_static_web_batches() {
    const HOST: &str = "com.example.static-host";
    const PAGE: &str = "com.example.static-page";
    let bed = Bed::new(false).await;
    let provider = PluginPackage::parse(
        &json!({
            "services": {"static": {"image": "test-only"}},
            "x-wonton": {"spec": "0.15", "id": HOST, "version": "1.0.0", "kind": "app",
                "title": {"en": "Static host"}, "description": {"en": "Static host"},
                "permissions": [], "provides": {"static_web": {
                    "service": "static", "port": 8080, "mount": "/srv/static"}}}
        })
        .to_string(),
        &langs(),
    )
    .unwrap();
    let files: StaticFiles = [("index.html".into(), b"page".to_vec())]
        .into_iter()
        .collect();
    let page = PluginPackage::parse(
        &json!({"x-wonton": {
            "spec": "0.15", "id": PAGE, "version": "1.0.0", "kind": "app",
            "runtime": "static_web", "title": {"en": "Page"},
            "description": {"en": "Page"}, "permissions": [], "webui": {"path": "/"}
        }})
        .to_string(),
        &langs(),
    )
    .unwrap()
    .with_static(files, &langs())
    .unwrap();
    bed.manager
        .install_package(provider, &[], langs())
        .await
        .unwrap();
    bed.manager
        .install_package(page, &[], langs())
        .await
        .unwrap();
    let hosted = bed.data.path().join("hosted/static_web").join(PAGE);
    assert!(hosted.join("index.html").is_file());
    let preview = bed.manager.uninstall_preview(&id(PAGE), &langs()).unwrap();
    let selected: Vec<_> = preview.unused_providers.into_iter().map(|p| p.id).collect();
    assert_eq!(selected, [HOST]);

    bed.manager
        .uninstall_detached(id(PAGE), true, selected, false, langs())
        .await
        .unwrap();

    assert!(bed.manager.record_for_test(&id(PAGE)).is_none());
    assert!(bed.manager.record_for_test(&id(HOST)).is_none());
    assert!(!hosted.exists());
    assert!(bed.docker.containers().is_empty());
}
