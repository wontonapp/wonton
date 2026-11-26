//! `wants: [models]` 声明：可选依赖、升级保留凭据、网关不在时也照样装。

use super::*;

/// 规范现在的写法：网关是 `wants: [models]`，不再写 `permissions`，也不用主人确认。
fn consumer_wanting_models() -> String {
    consumer().replace(
        "  permissions: [models]",
        "  permissions: []\n  wants: [models]",
    )
}

#[tokio::test]
pub(super) async fn wants_models_gets_variables_and_credentials() {
    let bed = bed().await;
    bed.manager
        .install(&consumer_wanting_models(), &[], super::langs())
        .await
        .unwrap();
    let env = web_env(&bed);
    assert_eq!(env["BASE"], "http://models.wonton.internal:8080/v1");
    assert_eq!(env["ANTHROPIC"], "http://models.wonton.internal:8080");
    assert_eq!(bed.keys.lock().unwrap()["app.test"], env["KEY"]);
    assert_eq!(env["KEY"].len(), 64);
    // `wants` 的使用方同样拿到 `models.wonton.internal` 的 hosts 别名。
    let created = bed
        .docker
        .created(&container_name("app-test", "web"))
        .unwrap();
    let gateway = bed
        .manager
        .engine
        .network_gateway("wonton-app-test")
        .await
        .unwrap();
    assert_eq!(
        created["HostConfig"]["ExtraHosts"],
        serde_json::json!([format!("models.wonton.internal:{gateway}")])
    );
}

/// 老清单升级到 `wants: [models]`：原来的凭据留着，不当成“新版本不要了”吊销。
#[tokio::test]
pub(super) async fn an_upgrade_from_the_old_permission_keeps_the_credential() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let key = web_env(&bed)["KEY"].clone();
    assert_eq!(bed.keys.lock().unwrap()["app.test"], key);
    let upgrade = PluginPackage::parse(
        &consumer_wanting_models().replace("version: 1.0.0", "version: 1.1.0"),
        &super::langs(),
    )
    .unwrap();
    bed.manager
        .upgrade_upload(
            &PluginId::parse("app.test").unwrap(),
            upgrade,
            Vec::new(),
            Vec::new(),
            false,
            &[],
            &Default::default(),
            &super::langs(),
        )
        .await
        .unwrap();
    assert_eq!(web_env(&bed)["KEY"], key, "升级到 wants 后凭据不变");
    assert_eq!(
        bed.keys.lock().unwrap()["app.test"],
        key,
        "网关里的那条没被吊销"
    );
}

/// 没有网关：`wants` 只是可选依赖，照样装，变量是空字符串，详情页一条轻提示。
#[tokio::test]
pub(super) async fn wants_without_gateway_installs_with_empty_variables() {
    let bed = bed_without_gateway().await;
    bed.manager
        .install(&consumer_wanting_models(), &[], super::langs())
        .await
        .unwrap();
    let env = web_env(&bed);
    for name in ["BASE", "ANTHROPIC", "KEY", "MODEL"] {
        assert_eq!(env[name], "", "{name}");
    }
    assert_eq!(bed.hints().await.hints.len(), 1);
}

/// Bot 服务按 Bot 签凭据，全插件服务和 `WONTON_MODEL_API_KEY` 都不落空。用 `wants` 判定。
#[tokio::test]
pub(super) async fn wants_models_bot_services_get_their_own_credential() {
    let bed = bed().await;
    let source = package(
        "  gateway:\n    image: hermes\n    environment:\n      KEY: ${WONTON_MODEL_API_KEY}\n  \
         web:\n    image: nginx\n    environment:\n      KEY: ${WONTON_MODEL_API_KEY}",
        "  bot: {display_name: {zh: 测试, en: Test}}\n  bots: {services: [gateway]}\n  \
         permissions: [matrix.bot]\n  wants: [models]",
    );
    let granted = vec!["matrix.bot".parse().unwrap()];
    bed.manager
        .install(&source, &granted, super::langs())
        .await
        .unwrap();
    let keys = bed.keys.lock().unwrap().clone();
    let gateway = env_of_container(
        &bed.docker,
        &Instance::default().bot_container("app-test", "gateway", "default"),
    );
    assert_eq!(gateway["KEY"], keys["app.test/default"]);
    assert_eq!(web_env(&bed)["KEY"], keys["app.test"]);
}
