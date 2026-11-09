//! `WONTON_HOST_NAME`：宿主主机名注入给插件，可用在环境变量和 `hostname` 上。

use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::manager::manager_with;
use super::{env_of_container, package};

async fn installed(name: Option<&str>) -> FakeDocker {
    let (docker, engine) = FakeDocker::start().await;
    if let Some(name) = name {
        docker.set_name(name);
    }
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(
        &package(
            "  web:\n    image: nginx\n    environment:\n      HOST: ${WONTON_HOST_NAME}\n    \
             hostname: ${WONTON_HOST_NAME}",
            "  permissions: []",
        ),
        &[],
        super::langs(),
    )
    .await
    .unwrap();
    docker
}

#[tokio::test]
async fn the_host_name_from_docker_info_reaches_the_container() {
    let docker = installed(Some("server-01")).await;
    let web = container_name("app-test", "web");
    let env = env_of_container(&docker, &web);
    assert_eq!(env.get("HOST").map(String::as_str), Some("server-01"));
    assert_eq!(docker.created(&web).unwrap()["Hostname"], "server-01");
}

/// 取不到时变量是空串而不是未定义：写了 `${WONTON_HOST_NAME}` 的插件照样装得上。
#[tokio::test]
async fn without_a_name_the_variable_is_empty_and_the_install_succeeds() {
    let docker = installed(None).await;
    let web = container_name("app-test", "web");
    let env = env_of_container(&docker, &web);
    assert_eq!(env.get("HOST").map(String::as_str), Some(""));
    assert_eq!(docker.created(&web).unwrap()["Hostname"], "");
}

/// 失败不缓存：启动时取不到注入空串，Docker 恢复后取到名字，之后装的插件也拿到它
/// （缓存住失败的话，这里会一直是空串）。
#[tokio::test]
async fn a_failed_lookup_is_retried_after_docker_recovers() {
    let (docker, engine) = FakeDocker::start().await;
    docker.set_name("server-01");
    docker.fail_info(Some("暂时连不上"));
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);

    // 核心启动时取：失败，这次注入空串。
    assert_eq!(m.host_name().await, "");

    docker.fail_info(None);
    m.install(
        &package(
            "  web:\n    image: nginx\n    environment:\n      HOST: ${WONTON_HOST_NAME}\n    \
             hostname: ${WONTON_HOST_NAME}",
            "  permissions: []",
        ),
        &[],
        super::langs(),
    )
    .await
    .unwrap();
    let web = container_name("app-test", "web");
    let env = env_of_container(&docker, &web);
    assert_eq!(env.get("HOST").map(String::as_str), Some("server-01"));
    assert_eq!(docker.created(&web).unwrap()["Hostname"], "server-01");
}
