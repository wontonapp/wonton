use rmcp::model::CallToolRequestParams;
use serde_json::json;
use wonton_protocol::UpgradeRequest;

use super::fixture::{Bed, REMOTE, SECRET, id, remote};
use crate::plugin::PluginPackage;
use crate::plugin::tests::langs;
use crate::plugin::upgrade::UpgradeTo;

async fn request(bed: &Bed, token: &str) -> UpgradeRequest {
    let mut document: serde_json::Value = serde_json::from_str(&remote("header").source).unwrap();
    document["x-wonton"]["version"] = json!("2.0.0");
    let package = PluginPackage::parse(&document.to_string(), &langs()).unwrap();
    let preview = bed
        .manager
        .preview_upgrade(&id(REMOTE), UpgradeTo::Upload(Box::new(package)), &langs())
        .await
        .unwrap();
    serde_json::from_value(json!({
        "source": preview.source, "digest": preview.digest, "grant": [],
        "secrets": {"token": token}
    }))
    .unwrap()
}

fn seed_timestamp(bed: &Bed) {
    let store = bed.manager.store.secrets();
    let values = store.load(&id(REMOTE)).unwrap().unwrap();
    store
        .save_changed(&id(REMOTE), &values, &[("token".into(), 123)].into())
        .unwrap();
}

#[tokio::test]
async fn rejected_remote_mcp_upgrade_restores_secret_bytes_and_old_runtime() {
    let mut bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    seed_timestamp(&bed);
    let path = bed
        .data
        .path()
        .join(format!("plugins/secrets/{REMOTE}.json"));
    let before = std::fs::read(&path).unwrap();
    let request = request(&bed, "rejected-new-key").await;
    *bed.fake.outcome.lock().unwrap() = "secret_rejected".into();
    let error = bed
        .manager
        .upgrade_source(&id(REMOTE), &request, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        bed.manager.record_for_test(&id(REMOTE)).unwrap().version,
        "1.0.0"
    );
    let store = bed.manager.store.secrets();
    assert_eq!(
        store.load(&id(REMOTE)).unwrap().unwrap()["token"],
        SECRET.as_bytes()
    );
    assert_eq!(store.changed(&id(REMOTE)).unwrap()["token"], 123);
    assert_eq!(
        bed.fake.probes.lock().unwrap().last().unwrap()["headers"]["authorization"],
        "Bearer rejected-new-key"
    );
    let agent = bed.agent().await;
    let result = agent
        .call_tool(CallToolRequestParams::new(format!(
            "{}__search",
            id(REMOTE).slug()
        )))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert!(
        serde_json::to_string(&result)
            .unwrap()
            .contains("remote-called")
    );
    {
        let calls = bed.fake.calls.lock().unwrap();
        assert!(!calls.is_empty());
        for (_, headers) in calls.iter() {
            assert_eq!(headers["authorization"], format!("Bearer {SECRET}"));
        }
    }
    agent.cancel().await.unwrap();
}

#[tokio::test]
async fn successful_remote_mcp_upgrade_commits_new_secret_and_timestamp() {
    let bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    seed_timestamp(&bed);
    let fresh = "accepted-new-key";
    let request = request(&bed, fresh).await;
    bed.manager
        .upgrade_source(&id(REMOTE), &request, &langs())
        .await
        .unwrap();
    assert_eq!(
        bed.manager.record_for_test(&id(REMOTE)).unwrap().version,
        "2.0.0"
    );
    let store = bed.manager.store.secrets();
    assert_eq!(
        store.load(&id(REMOTE)).unwrap().unwrap()["token"],
        fresh.as_bytes()
    );
    assert!(store.changed(&id(REMOTE)).unwrap()["token"] > 123);
    assert_eq!(
        bed.fake.probes.lock().unwrap().last().unwrap()["headers"]["authorization"],
        format!("Bearer {fresh}")
    );
    let upstreams = crate::mcp::upstream::Upstreams::new();
    let server = bed.manager.mcp_servers().pop().unwrap();
    upstreams
        .call(&server, "search", None, std::future::pending())
        .await
        .unwrap();
    assert_eq!(
        bed.fake.calls.lock().unwrap().last().unwrap().1["authorization"],
        format!("Bearer {fresh}")
    );
}

#[tokio::test]
async fn remote_mcp_upgrade_does_not_overwrite_an_unreadable_secret_store() {
    let bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    let request = request(&bed, "must-not-be-written").await;
    let path = bed
        .data
        .path()
        .join(format!("plugins/secrets/{REMOTE}.json"));
    std::fs::write(&path, b"invalid secret JSON").unwrap();
    let probes = bed.fake.probes.lock().unwrap().len();
    bed.manager
        .upgrade_source(&id(REMOTE), &request, &langs())
        .await
        .unwrap_err();
    assert_eq!(std::fs::read(&path).unwrap(), b"invalid secret JSON");
    assert_eq!(bed.fake.probes.lock().unwrap().len(), probes);
    assert_eq!(
        bed.manager.record_for_test(&id(REMOTE)).unwrap().version,
        "1.0.0"
    );
}
