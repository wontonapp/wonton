//! 第三方客户端模型：public/lan crt-list 的内容、公开访问开关、重载失败时回滚。

use super::test_support::*;

#[tokio::test]
async fn open_lan_writes_lan_crt_list_and_reloads() {
    let rig = Rig::new().await;
    // 入口只在实际选出了局域网地址时才建转发容器。
    let address = crate::plugin::lan::entry_address::EntryAddress {
        address: "192.0.2.10".parse().unwrap(),
        subnet: "192.0.2.0/24".parse().unwrap(),
        publish: "192.0.2.10".parse().unwrap(),
    };
    rig.containers
        .set_lan(crate::clients::lan_forward::LanState::Selected {
            address,
            record: crate::clients::lan_forward::Record::Ok,
        });
    let before = rig.docker.writes().len();
    rig.containers
        .apply_clients_model(
            &settings(),
            true,
            vec!["public-app".into()],
            vec!["lan-app".into()],
        )
        .await
        .unwrap();
    let lan = std::fs::read_to_string(rig.dir.path().join("entry/certs/lan.crt-list")).unwrap();
    let public =
        std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list")).unwrap();
    assert!(lan.contains("lan-app.lan.k3f9qzab.example.test"), "{lan}");
    assert!(
        public.contains("public-app.k3f9qzab.example.test"),
        "{public}"
    );
    assert!(rig.writes_for("/kill").is_empty());
    // 转发容器要先于入口重载/接线建好：入口的 `frontend lan` 绑它的名字，先写配置会让新 worker 卡 bind。
    let writes = &rig.docker.writes()[before..];
    let created = writes
        .iter()
        .position(|write| {
            write.contains("/containers/create") && write.contains("name=wonton-lan-forward")
        })
        .unwrap_or_else(|| panic!("第一个建的容器不是局域网转发：{writes:?}"));
    if let Some(connected) = writes
        .iter()
        .position(|write| write.contains("/networks/") && write.contains("/connect"))
    {
        assert!(created < connected, "{writes:?}");
    }
}

#[tokio::test]
async fn public_model_respects_public_access_gate() {
    let rig = Rig::new().await;
    rig.containers
        .apply_clients_model(&settings(), false, vec!["app".into()], vec![])
        .await
        .unwrap();
    let path = rig.dir.path().join("entry/certs/public.crt-list");
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("app.k3f9qzab.example.test")
    );
    rig.containers
        .apply_clients_model(&settings(), true, vec!["app".into()], vec![])
        .await
        .unwrap();
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("app.k3f9qzab.example.test")
    );
}

#[tokio::test]
async fn reload_failure_restores_all_configuration() {
    let mut rig = Rig::new().await;
    rig.containers
        .apply_clients_model(&settings(), true, vec!["old".into()], vec![])
        .await
        .unwrap();
    let entry = rig.dir.path().join("entry");
    let paths = ["haproxy.cfg", "certs/public.crt-list", "certs/lan.crt-list"];
    let before: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(entry.join(path)).unwrap())
        .collect();
    rig.containers.reload_addr = master_forever("Success=0\n--\nUSR2 failed\n").await;
    let error = rig
        .containers
        .apply_clients_model(&settings(), true, vec!["new".into()], vec![])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("USR2 failed"), "{error}");
    for (path, expected) in paths.iter().zip(before) {
        assert_eq!(std::fs::read(entry.join(path)).unwrap(), expected, "{path}");
    }
    assert!(rig.writes_for("/kill").is_empty());
}

#[tokio::test]
async fn reconcile_does_not_replace_client_crt_lists() {
    let rig = Rig::new().await;
    rig.containers
        .apply_clients_model(&settings(), true, vec!["app".into()], vec![])
        .await
        .unwrap();
    let path = rig.dir.path().join("entry/certs/public.crt-list");
    let before = std::fs::read(&path).unwrap();
    rig.reconcile(Some(&settings())).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), before);
}
