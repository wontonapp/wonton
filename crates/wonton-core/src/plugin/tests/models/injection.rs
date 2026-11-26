//! 给使用方签发凭据、注入模型变量：按使用方一份，Bot 服务按 Bot 一份。

use super::*;

#[tokio::test]
pub(super) async fn consumers_get_their_own_verified_credential_through_the_core() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = web_env(&bed);
    assert_eq!(env["BASE"], "http://models.wonton.internal:8080/v1");
    assert_eq!(env["ANTHROPIC"], "http://models.wonton.internal:8080");
    assert_eq!(env["MODEL"], "group/default");
    assert_eq!(bed.keys.lock().unwrap()["app.test"], env["KEY"]);
    assert_eq!(env["KEY"].len(), 64);
    // 值走标准输入，不进命令行参数。健康探测凭据（<网关>/health）也在这之后签发，按名挑使用方的。
    let issue = bed
        .docker
        .execs()
        .into_iter()
        .rev()
        .find(|exec| exec.cmd.last().is_some_and(|name| name == "app.test"))
        .unwrap();
    assert_eq!(issue.container, container_name("system-models", "magpie"));
    assert!(!issue.cmd.iter().any(|arg| arg.contains(&env["KEY"])));
    assert_eq!(issue.stdin, format!("{}\n", env["KEY"]).into_bytes());
    // 核心直接跑在宿主上：插件经自己网络的网关访问 models.wonton.internal。
    let created = bed
        .docker
        .created(&container_name("app-test", "web"))
        .unwrap();
    let hosts = created["HostConfig"]["ExtraHosts"].clone();
    let gateway = bed
        .manager
        .engine
        .network_gateway("wonton-app-test")
        .await
        .unwrap();
    assert_eq!(
        hosts,
        serde_json::json!([format!("models.wonton.internal:{gateway}")])
    );
}

#[tokio::test]
pub(super) async fn bot_services_get_the_bot_credential_and_shared_services_the_plugin_one() {
    let bed = bed().await;
    let source = package(
        "  gateway:\n    image: hermes\n    environment:\n      KEY: ${WONTON_MODEL_API_KEY}\n  \
         web:\n    image: nginx\n    environment:\n      KEY: ${WONTON_MODEL_API_KEY}",
        "  bot: {display_name: {zh: 测试, en: Test}}\n  bots: {services: [gateway]}\n  \
         permissions: [matrix.bot, models]",
    );
    let granted = vec!["matrix.bot".parse().unwrap(), "models".parse().unwrap()];
    bed.manager
        .install(&source, &granted, super::langs())
        .await
        .unwrap();
    let keys = bed.keys.lock().unwrap().clone();
    // gateway 是 Bot 服务：容器按 Bot 一份。
    let gateway = env_of_container(
        &bed.docker,
        &Instance::default().bot_container("app-test", "gateway", "default"),
    );
    assert_eq!(gateway["KEY"], keys["app.test/default"]);
    assert_eq!(web_env(&bed)["KEY"], keys["app.test"]);
    assert_ne!(keys["app.test"], keys["app.test/default"]);
}
