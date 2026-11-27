//! 准备中的签发：同名保存不动它，换服务商要等它结束。

use super::*;

fn registered_settings(dir: &Path) {
    settings(dir, Some("abcdefgh.example.test"));
    let path = dir.join("entry/settings.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let credentials = serde_json::json!({
        "username": "test", "password": "test", "subdomain": "test",
        "fulldomain": "test.auth.example.test"
    });
    value["registration"] = serde_json::json!({
        "home": "abcdefgh", "domain": "abcdefgh.example.test", "frp_token": "test",
        "acme_dns": credentials, "lan_acme_dns": credentials,
        "acme_dns_url": "https://auth.example.test",
        "relay": {"host": "relay.example.test", "port": 7000, "server_name": "relay.example.test"},
        "suspended": false
    });
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
#[tokio::test]
async fn registered_unchanged_save_preserves_prepared_issuance() {
    let dir = tempfile::tempdir().unwrap();
    registered_settings(dir.path());
    let clients = domain(dir.path());
    let mut put = request(Some("abcdefgh.example.test"));
    put.provider.clear();
    put.credentials.clear();
    let saved = clients.save(put).await.unwrap();
    assert!(!saved.provider_changed);
    let prepared = dir.path().join("entry/prepared.json");
    fs::write(&prepared, "in-progress").unwrap();
    let mut put = request(Some("abcdefgh.example.test"));
    put.provider.clear();
    put.credentials.clear();
    let saved = clients.save(put).await.unwrap();
    assert!(!saved.provider_changed);
    assert_eq!(fs::read_to_string(prepared).unwrap(), "in-progress");
}
#[tokio::test]
async fn provider_change_waits_for_inflight_issuance() {
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients.save(request(None)).await.unwrap();
    let serial = clients.issuance.lock().await;
    let prepared = dir.path().join("entry/prepared.json");
    fs::write(&prepared, "in-progress").unwrap();
    let mut put = request(None);
    put.provider = "otherdns".into();
    let saving = clients.save(put);
    tokio::pin!(saving);
    if tokio::time::timeout(Duration::from_millis(150), saving.as_mut())
        .await
        .is_ok()
    {
        panic!("签发进行中 save 不该删准备状态、不该完成");
    }
    assert_eq!(fs::read_to_string(&prepared).unwrap(), "in-progress");
    drop(serial);
    assert!(saving.await.unwrap().provider_changed);
    assert!(!prepared.exists());
}
