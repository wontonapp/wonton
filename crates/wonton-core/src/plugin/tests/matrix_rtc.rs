//! `matrix_rtc` 的提供方（ADR-0096）：拿到服务器名、网络里有服务器名的核心别名、同一时间只有一个。

use super::super::fake_docker::FakeDocker;
use super::super::{PluginManager, PluginPackage};
use super::manager::{manager_in_container, manager_with};
use super::{env_of_container, package};

const SERVER: &str = "home.test";

fn provider(id: &str) -> String {
    package(
        "  jwt:\n    image: nginx\n    environment:\n      FULL_ACCESS: ${WONTON_MATRIX_SERVER_NAME}",
        "  permissions: []\n  provides:\n    matrix_rtc: {service: jwt, port: 8080}",
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", &format!("id: {id}"))
}

async fn install(m: &PluginManager, compose: &str) -> Result<(), String> {
    let grants = PluginPackage::parse(compose, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    m.install(compose, &grants, super::langs())
        .await
        .map(|_| ())
        .map_err(|e| e.kind().to_owned())
}

#[tokio::test]
async fn the_provider_gets_the_server_name_and_resolves_it_to_the_core() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir).with_matrix_server_name(SERVER);
    install(&m, &provider("app.call")).await.unwrap();
    let env = env_of_container(&docker, "wonton-app-call--jwt");
    assert_eq!(env["FULL_ACCESS"], SERVER);
    // 核心跑在宿主上：服务器名经插件网络的网关到核心。
    let gateway = m.engine.network_gateway("wonton-app-call").await.unwrap();
    let created = docker.created("wonton-app-call--jwt").unwrap();
    let hosts = created["HostConfig"]["ExtraHosts"].as_array().unwrap();
    assert!(
        hosts.contains(&format!("{SERVER}:{gateway}").into()),
        "{hosts:?}"
    );
}

#[tokio::test]
async fn in_a_container_the_core_takes_the_server_name_as_an_alias() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    // 服务器名带端口时别名只用主机名。
    let m = manager_in_container(engine, &dir).with_matrix_server_name("home.test:8448");
    install(&m, &provider("app.call")).await.unwrap();
    assert!(
        docker
            .aliases("wonton-app-call")
            .contains(&SERVER.to_owned()),
        "{:?}",
        docker.aliases("wonton-app-call")
    );
    let env = env_of_container(&docker, "wonton-app-call--jwt");
    assert_eq!(env["FULL_ACCESS"], "home.test:8448");
}

#[tokio::test]
async fn only_one_provider_at_a_time_and_only_with_matrix() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine.clone(), &dir).with_matrix_server_name(SERVER);
    install(&m, &provider("app.call")).await.unwrap();
    assert_eq!(
        install(&m, &provider("app.call.other")).await.unwrap_err(),
        "conflict"
    );

    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    assert_eq!(
        install(&m, &provider("app.call")).await.unwrap_err(),
        "matrix_unavailable"
    );
}
