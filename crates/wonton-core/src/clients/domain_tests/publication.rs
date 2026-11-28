//! 证书发布顺序：先写入口配置再重载。

use super::*;

/// 调试用：把域名设置写成这个后缀；`renewable` 时连签发要用的服务商、凭据和条款一起写。
fn app_domain(dir: &Path, suffix: &str, renewable: bool) {
    let mut value = serde_json::json!({"suffix": suffix});
    if renewable {
        value["provider"] = serde_json::json!("httpreq");
        value["credentials"] =
            serde_json::json!({"HTTPREQ_ENDPOINT": "https://relay.test/present"});
        value["terms_accepted"] = serde_json::json!(true);
    }
    fs::write(dir.join("entry/domain.json"), value.to_string()).unwrap();
}
/// 假 lego 只会“成功”：账号和它要发布的新证书自己放。
fn fake_lego_outputs(dir: &Path) {
    let account = dir.join("entry/acme/accounts/pebble.test_14000/wonton");
    fs::create_dir_all(&account).unwrap();
    fs::write(
        account.join("account.json"),
        serde_json::json!({
            "server": "https://pebble.test:14000/dir",
            "registration": {
                "status": "valid",
                "accountURL": "https://pebble.test:14000/my-account/1"
            }
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        account.join("wonton.key"),
        rcgen::KeyPair::generate().unwrap().serialize_pem(),
    )
    .unwrap();
    let new = rcgen::generate_simple_self_signed(vec![
        "example.test".to_owned(),
        "*.example.test".to_owned(),
    ])
    .unwrap();
    fs::create_dir_all(dir.join("entry/acme/certificates")).unwrap();
    fs::write(dir.join("entry/acme/certificates/site.crt"), new.cert.pem()).unwrap();
}
/// 读到换行为止：假的主进程只等一条 `reload` 命令。
async fn read_command(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    let mut byte = [0u8; 1];
    loop {
        if socket.read_exact(&mut byte).await.is_err() || byte[0] == b'\n' {
            break;
        }
    }
}
/// 假的管理端口：每收到一次 reload 就把当时的 crt-list 记一条，立刻回成功。
async fn recording_master(
    crt_list: std::path::PathBuf,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::sync::{Arc, Mutex};
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let records: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = records.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let seen = seen.clone();
            let crt_list = crt_list.clone();
            tokio::spawn(async move {
                read_command(&mut socket).await;
                let now = match std::fs::read_to_string(&crt_list) {
                    Ok(content) => content,
                    Err(error) => format!("<{error}>"),
                };
                seen.lock().expect("重载记录锁").push(now);
                let _ = socket.write_all(b"Success=1\n").await;
            });
        }
    });
    (addr, records)
}
/// 入口按旧证书跑着、后缀已经改成父域、假 lego 的产物也摆好了：签发前的现场。
async fn renamed_entry_rig() -> (
    FakeDocker,
    std::sync::Arc<EntryContainers>,
    std::sync::Arc<Domain>,
    tempfile::TempDir,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::sync::Arc;
    let (docker, engine) = FakeDocker::start().await;
    docker.add_stranger_container(
        "wonton-core",
        serde_json::json!({"Labels": {"com.docker.compose.project": "wonton"}}),
    );
    for network in ["clients", "edge", "relay"] {
        docker.add_unlabeled_network(&format!("wonton_{network}"));
    }
    let dir = tempfile::tempdir().unwrap();
    let (reload_addr, reloads) =
        recording_master(dir.path().join("entry/certs/public.crt-list")).await;
    let entry = Arc::new(
        EntryContainers::new(
            engine.clone(),
            dir.path().to_owned(),
            dir.path().join("data"),
            crate::plugin::names::Instance::default(),
            Some("wonton-core".to_owned()),
        )
        .with_binding("127.0.0.1", "18443")
        .with_reload_addr(reload_addr),
    );
    let clients = Domain::new(
        dir.path().to_owned(),
        dir.path().join("data"),
        engine.clone(),
        None,
    );
    let old = rcgen::generate_simple_self_signed(vec!["old.example.test".to_owned()]).unwrap();
    fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
    fs::write(dir.path().join("entry/certs/site.pem"), old.cert.pem()).unwrap();
    fs::write(
        dir.path().join("entry/certs/site.key"),
        old.signing_key.serialize_pem(),
    )
    .unwrap();
    app_domain(dir.path(), "old.example.test", false);
    settings(dir.path(), Some("old.example.test"));
    entry.reconcile(None, true).await.unwrap();
    // 后缀改成父域。
    settings(dir.path(), Some("example.test"));
    app_domain(dir.path(), "example.test", true);
    fake_lego_outputs(dir.path());
    (docker, entry, clients, dir, reloads)
}
/// 签发成功后先按新证书写入口配置、再让入口重读：反了入口会拿旧名字的 crt-list 去读新证书，
/// 旧名字被 `strict-sni` 拒、新名字证书对不上。入口收到重载的那一刻就得看到新名字。
#[tokio::test]
async fn a_published_certificate_writes_the_config_before_reloading() {
    use std::sync::atomic::Ordering;
    let (_docker, entry, clients, dir, reloads) = renamed_entry_rig().await;
    assert!(
        reloads.lock().expect("重载记录锁").is_empty(),
        "准备阶段不该重载：{:?}",
        reloads.lock().expect("重载记录锁")
    );
    let served = fs::read_to_string(dir.path().join("entry/certs/public.crt-list")).unwrap();
    assert!(served.contains("old.example.test"), "{served}");
    // 签发成功只叫醒对账，自己不重载。
    clients.renew_if_configured();
    for _ in 0..20_000 {
        if clients.pending.load(Ordering::SeqCst) == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(clients.pending.load(Ordering::SeqCst), 0);
    assert!(
        reloads.lock().expect("重载记录锁").is_empty(),
        "签发成功不该自己先重载"
    );
    // 循环收到唤醒就做这一轮：先按新证书写配置，再重载。重载那一刻看到的名字必须已经是新的。
    entry.reconcile(None, true).await.unwrap();
    let seen = reloads.lock().expect("重载记录锁").clone();
    let last = seen.last().expect("入口没收到重载");
    let names: Vec<&str> = last.split_whitespace().collect();
    assert!(names.contains(&"example.test"), "记录：{seen:?}");
    assert!(!names.contains(&"old.example.test"), "记录：{seen:?}");
}
