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

/// 证书还新、给的就是这个名字、上一次没失败：再保存一次不该重新签一张。
/// 换名字时旧证书虽然新也不算数（`reset_prepared` 已经把身份删了）。
#[tokio::test]
async fn a_fresh_certificate_skips_the_repeat_issue() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("example.test"));
    let clients = domain(dir.path());
    clients
        .apply(|store| {
            *store = super::Store {
                suffix: Some("example.test".into()),
                provider: Some("httpreq".into()),
                credentials: [("HTTPREQ_TOKEN".into(), "a-token".into())].into(),
                env_file: None,
                terms_accepted: true,
                last_error: None,
            };
            Ok(())
        })
        .await
        .unwrap();
    certificate(dir.path());
    prepared(dir.path(), "example.test", "httpreq");
    let snapshot = clients.effective().unwrap();
    assert_eq!(snapshot.suffix.as_deref(), Some("example.test"));
    assert!(snapshot.last_error.is_none());
    let certs = dir.path().join("entry/certs");
    assert!(certs.join("site.key").is_file(), "key 没写出来");
    let names = crate::entry_certificate::published_names(&certs.join("site.pem"));
    assert_eq!(names, ["example.test"], "SAN 没读到");
    assert!(
        clients
            .not_after_ms()
            .is_some_and(|at| at > super::now_ms() + 30 * 86_400_000),
        "到期时间不够远：{:?}",
        clients.not_after_ms()
    );
    assert!(clients.fresh(&snapshot), "先决条件就该是新证书");
    let quiet = clients.engine.recorded_creations().len();
    clients.pending.fetch_add(1, Ordering::SeqCst);
    clients
        .clone()
        .run_issue(clients.effective().unwrap(), false)
        .await;
    assert_eq!(
        clients.engine.recorded_creations().len(),
        quiet,
        "证书还新且覆盖这个名字，不该再起 lego"
    );
    // 换了名字：旧的 SAN 不覆盖，必须重新签。
    clients
        .update(|store| store.suffix = Some("neu.example.test".into()))
        .await
        .unwrap();
    // 真实的 save() 会在这里删准备状态；这里走 update()，补一下。
    clients.reset_prepared();
    clients.pending.fetch_add(1, Ordering::SeqCst);
    clients
        .clone()
        .run_issue(clients.effective().unwrap(), false)
        .await;
    assert_eq!(
        clients.engine.recorded_creations().len(),
        quiet + 1,
        "换名字后该照常走签发"
    );
}

/// 证书还新，但换服务商（准备状态被删）或只换凭据：保存必须真的去签一次，验新配置。
#[tokio::test]
async fn provider_or_credential_change_with_a_fresh_certificate_still_issues() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("example.test"));
    let clients = domain(dir.path());
    clients
        .apply(|store| {
            *store = super::Store {
                suffix: Some("example.test".into()),
                provider: Some("httpreq".into()),
                credentials: [("HTTPREQ_TOKEN".into(), "a-token".into())].into(),
                env_file: None,
                terms_accepted: true,
                last_error: None,
            };
            Ok(())
        })
        .await
        .unwrap();
    certificate(dir.path());
    prepared(dir.path(), "example.test", "httpreq");
    let settle = async || {
        for _ in 0..20_000 {
            if clients.pending.load(Ordering::SeqCst) == 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("签发没在限定的轮数里结束");
    };
    // 只换凭据：准备状态还在，也要强制签一次。
    let quiet = clients.engine.recorded_creations().len();
    let mut put = request(None);
    put.credentials = vec![CredentialEntry {
        name: "HTTPREQ_TOKEN".into(),
        value: SecretString::new("b-token"),
    }];
    clients.clone().save(put).await.unwrap();
    settle().await;
    assert!(
        clients.engine.recorded_creations().len() > quiet,
        "只换凭据也得验一次新凭据"
    );
    // 换服务商：reset_prepared 删掉 prepared.json，fresh 不再成立。
    let quiet = clients.engine.recorded_creations().len();
    let mut put = request(None);
    put.provider = "otherdns".into();
    clients.clone().save(put).await.unwrap();
    settle().await;
    assert!(
        clients.engine.recorded_creations().len() > quiet,
        "换服务商必须重新签"
    );
}
