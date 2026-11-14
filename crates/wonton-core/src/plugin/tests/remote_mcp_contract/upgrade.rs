use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use axum::extract::State;
use axum::routing::post;
use tokio::sync::Notify;

use rmcp::model::CallToolRequestParams;
use serde_json::json;
use wonton_protocol::UpgradeRequest;

use super::fixture::{Bed, Fake, REMOTE, SECRET, id, remote};
use crate::plugin::PluginPackage;
use crate::plugin::tests::langs;
use crate::plugin::upgrade::UpgradeTo;

async fn request(bed: &Bed, token: &str) -> UpgradeRequest {
    let mut document: serde_json::Value = serde_json::from_str(&remote("header").source).unwrap();
    document["x-wonton"]["version"] = json!("2.0.0");
    let package = PluginPackage::parse(&document.to_string(), &langs()).unwrap();
    request_package(bed, package, token).await
}

async fn request_package(bed: &Bed, package: PluginPackage, token: &str) -> UpgradeRequest {
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

async fn assert_runtime_token(bed: &Bed, token: &str) {
    let upstreams = crate::mcp::upstream::Upstreams::new();
    let server = bed.manager.mcp_servers().pop().unwrap();
    upstreams
        .call(&server, "search", None, std::future::pending())
        .await
        .unwrap();
    assert_eq!(
        bed.fake.calls.lock().unwrap().last().unwrap().1["authorization"],
        format!("Bearer {token}")
    );
}

#[tokio::test]
async fn rejected_same_digest_source_change_restores_exact_old_state() {
    let bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    seed_timestamp(&bed);
    let old_record = bed.manager.record_for_test(&id(REMOTE)).unwrap();
    let path = bed
        .data
        .path()
        .join(format!("plugins/secrets/{REMOTE}.json"));
    let before = std::fs::read(&path).unwrap();
    let mut package = remote("header");
    package.origin = Some(
        serde_json::from_value(json!({
            "repo": "github:example/relocated-tools", "commit": "a".repeat(40)
        }))
        .unwrap(),
    );
    let digest = package.digest();
    assert_eq!(digest, remote("header").digest());
    assert_ne!(package.origin, old_record.origin);
    let request = request_package(&bed, package, "rejected-source-key").await;
    assert_eq!(request.digest, digest);
    *bed.fake.outcome.lock().unwrap() = "secret_rejected".into();
    let error = bed
        .manager
        .upgrade_source(&id(REMOTE), &request, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let store = bed.manager.store.secrets();
    assert_eq!(
        store.load(&id(REMOTE)).unwrap().unwrap()["token"],
        SECRET.as_bytes()
    );
    assert_eq!(store.changed(&id(REMOTE)).unwrap()["token"], 123);
    let restored = bed.manager.record_for_test(&id(REMOTE)).unwrap();
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(&old_record).unwrap()
    );
    assert_eq!(
        bed.fake.probes.lock().unwrap().last().unwrap()["headers"]["authorization"],
        "Bearer rejected-source-key"
    );
    assert_runtime_token(&bed, SECRET).await;
}

#[derive(Clone)]
struct ProbeGate {
    fake: Fake,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

async fn gated_probe(
    State(gate): State<ProbeGate>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    gate.fake.probes.lock().unwrap().push(body.clone());
    let outcome = if body["headers"]["authorization"] == "Bearer staged-key-B" {
        gate.entered.notify_one();
        gate.release.notified().await;
        "secret_rejected"
    } else {
        "ok"
    };
    axum::Json(json!({"outcome": outcome, "http_status": 401, "detail": "probe gate"}))
}

#[tokio::test]
async fn remote_mcp_upgrade_rollback_serializes_a_confirmed_secret_update() {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let bed = Bed::with_connector(|fake, base| {
        base.merge(
            axum::Router::new()
                .route("/_wonton/probe", post(gated_probe))
                .with_state(ProbeGate {
                    fake,
                    entered: entered.clone(),
                    release: release.clone(),
                }),
        )
    })
    .await;
    bed.install("header").await.unwrap();
    seed_timestamp(&bed);
    let path = bed
        .data
        .path()
        .join(format!("plugins/secrets/{REMOTE}.json"));
    let before = std::fs::read(&path).unwrap();
    let request = request(&bed, "staged-key-B").await;
    let manager = bed.manager.clone();
    let upgrade = tokio::spawn(async move {
        manager
            .upgrade_source(&id(REMOTE), &request, &langs())
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .unwrap();
    let staged = std::fs::read(&path).unwrap();
    assert_ne!(staged, before);
    assert_eq!(
        bed.manager
            .store
            .secrets()
            .load(&id(REMOTE))
            .unwrap()
            .unwrap()["token"],
        b"staged-key-B"
    );
    let plugin = id(REMOTE);
    let languages = langs();
    let mut setter = Box::pin(bed.manager.set_secret_input_inner(
        &plugin,
        "token",
        Some("confirmed-key-C"),
        &languages,
    ));
    // 直接轮询真正的事务 future，不靠 spawn 调度或睡眠猜测 C 是否走到写入处。
    std::future::poll_fn(|cx| {
        assert!(setter.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(std::fs::read(&path).unwrap(), staged);
    release.notify_one();
    let error = tokio::time::timeout(Duration::from_secs(10), upgrade)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    tokio::time::timeout(Duration::from_secs(10), setter)
        .await
        .unwrap()
        .unwrap();
    let store = bed.manager.store.secrets();
    assert_eq!(
        store.load(&plugin).unwrap().unwrap()["token"],
        b"confirmed-key-C"
    );
    assert!(store.changed(&plugin).unwrap()["token"] > 123);
    assert_eq!(
        bed.manager.record_for_test(&plugin).unwrap().version,
        "1.0.0"
    );
    assert_runtime_token(&bed, "confirmed-key-C").await;
}
