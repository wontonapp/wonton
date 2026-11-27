//! 创建请求：容器、网络、别名、安全选项、两种连接器、发布端口。

use std::collections::HashMap;

use serde_json::{Value, json};

use super::super::{
    CONNECTOR_RELOAD_ADDR, ENTRY_CLIENTS_ALIAS, ENTRY_EDGE_ALIAS, ENTRY_RELOAD_ADDR,
};
use super::test_support::*;
use crate::entry_compose::HAPROXY_IMAGE;
use crate::entry_runtime::FRPC_IMAGE;

/// 有联邦：入口容器和连接器都建起来，接在网络里。
#[tokio::test]
async fn federation_settings_create_entry_and_connector() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let entry = rig.created("wonton-entry");
    let connector = rig.created("wonton-connector");
    assert_eq!(entry["Image"], HAPROXY_IMAGE);
    assert_eq!(
        entry["Cmd"],
        json!([
            "haproxy",
            "-W",
            "-db",
            "-f",
            "/config/haproxy.cfg",
            "-S",
            ENTRY_RELOAD_ADDR
        ])
    );
    assert_eq!(connector["Image"], HAPROXY_IMAGE);
    assert_eq!(
        connector["Cmd"],
        json!([
            "haproxy",
            "-W",
            "-db",
            "-f",
            "/config/connector.cfg",
            "-S",
            CONNECTOR_RELOAD_ADDR
        ])
    );
    // 入口在 edge 和 clients 上，连接器在 edge 和 relay 上。
    let edge = format!("{PROJECT}_edge");
    let clients = format!("{PROJECT}_clients");
    let relay = format!("{PROJECT}_relay");
    assert_eq!(
        rig.docker.endpoint(&clients, "wonton-entry").unwrap()["Aliases"],
        json!([ENTRY_CLIENTS_ALIAS])
    );
    assert_eq!(
        rig.docker.endpoint(&edge, "wonton-entry").unwrap()["Aliases"],
        json!([ENTRY_EDGE_ALIAS])
    );
    assert!(rig.docker.endpoint(&relay, "wonton-entry").is_none());
    assert!(rig.docker.endpoint(&clients, "wonton-connector").is_none());
    assert!(rig.docker.endpoint(&relay, "wonton-connector").is_some());
    assert_eq!(rig.docker.gateway_priority(&edge), Some(-1));
    // 端点写在创建请求里，不走 `network_mode: none` 再 connect（Docker 拒绝那种接法）。
    assert_ne!(entry["HostConfig"]["NetworkMode"], json!("none"));
    assert!(entry["NetworkingConfig"]["EndpointsConfig"][&clients].is_object());
    assert!(entry["NetworkingConfig"]["EndpointsConfig"][&edge].is_object());
}

/// 安全选项逐项断言：非 root、只读根、能力全去、不升权、清代理变量、没有 Docker socket 挂载。
#[tokio::test]
async fn containers_have_security_options() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let user = std::fs::metadata(rig.dir.path().join("entry/certs/site.pem"))
        .map(|meta| {
            use std::os::unix::fs::MetadataExt;
            format!("{}:{}", meta.uid(), meta.gid())
        })
        .unwrap();
    for name in ["wonton-entry", "wonton-connector"] {
        let created = rig.created(name);
        let host = &created["HostConfig"];
        assert_eq!(created["User"], json!(user), "{name} 的用户");
        assert_eq!(host["ReadonlyRootfs"], json!(true), "{name}");
        assert_eq!(host["CapDrop"], json!(["ALL"]), "{name}");
        assert_eq!(
            host["SecurityOpt"],
            json!(["no-new-privileges:true"]),
            "{name}"
        );
        let env: HashMap<&str, &str> = created["Env"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item.as_str()?.split_once('='))
            .collect();
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            assert_eq!(env.get(name), Some(&""), "{name}");
        }
        let binds: Vec<&str> = host["Binds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            binds.iter().all(|bind| !bind.contains("docker.sock")),
            "{binds:?}"
        );
        assert_eq!(host["LogConfig"]["Config"]["max-size"], "10m");
        assert_eq!(host["LogConfig"]["Config"]["max-file"], "3");
        // Docker 重启后要自己回来：核心发过信号的容器会被记成手动停过，`unless-stopped` 不再拉起。
        assert_eq!(host["RestartPolicy"]["Name"], "always");
        assert_eq!(created["StopSignal"], "SIGTERM");
    }
    // 连接器不在 core 网络上；入口也不在。
    let core = format!("{PROJECT}_core");
    assert!(rig.docker.endpoint(&core, "wonton-connector").is_none());
    assert!(rig.docker.endpoint(&core, "wonton-entry").is_none());
}

/// 家名服务（中继）时连接器是 frpc，读它的配置和 frps 的 CA。
#[tokio::test]
async fn federation_registration_uses_the_relay_connector() {
    let rig = Rig::new().await;
    let mut settings = settings();
    settings.registration = Some(federation_registration());
    let ca = rig.dir.path().join("relay-ca.pem");
    std::fs::write(&ca, "ca").unwrap();
    settings.relay_ca = Some(ca);
    rig.reconcile(Some(&settings)).await.unwrap();
    let created = rig.created("wonton-connector");
    assert_eq!(created["Image"], FRPC_IMAGE);
    assert_eq!(created["Cmd"], json!(["-c", "/config/frpc.json"]));
    let binds: Vec<&str> = created["HostConfig"]["Binds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        binds.iter().any(|bind| bind.contains("frpc.json")),
        "{binds:?}"
    );
    assert!(
        binds.iter().any(|bind| bind.contains("relay-ca.pem")),
        "{binds:?}"
    );
    assert_eq!(created["HostConfig"]["PortBindings"], Value::Null);
}

/// 路由器转发：连接器发布到宿主（默认只有回环）。
#[tokio::test]
async fn router_forwarding_publishes_the_host_port() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let created = rig.created("wonton-connector");
    assert_eq!(
        created["HostConfig"]["PortBindings"]["8443/tcp"],
        json!([{"HostIp": "127.0.0.1", "HostPort": "8443"}])
    );
}
