//! Bot 插件升级：设备、令牌和出口代理在升级与回滚里都不变。

use std::sync::Arc;

use super::super::egress::settings::StoredEgress;
use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{PluginId, PluginPackage, PluginStatus};
use super::env_of_container;
use super::manager::manager_in_container;
use super::upgrade::{echo, image, permissions, upgrade_expect_rollback};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::start;

#[tokio::test]
async fn a_bot_keeps_its_device_and_proxy_when_upgraded_and_when_rolled_back() {
    let (docker, engine) = FakeDocker::start().await;
    let (tuwunel, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
        .await
        .unwrap();
    let m = Arc::new(manager_in_container(engine, &dir).with_matrix(Some(Arc::new(matrix))));
    let settings = StoredEgress {
        url: Some("http://proxy:1".to_owned()),
        direct_cidrs: Vec::new(),
        use_plugin: false,
    };
    m.store.save_egress_settings(&settings).unwrap();
    let id = PluginId::parse("app.echo").unwrap();
    let bot = container_name("app-echo", "bot");
    let v1 = echo("1.0.0", "");
    m.install(&v1, &permissions(&v1), super::langs())
        .await
        .unwrap();
    let env = env_of_container(&docker, &bot);
    let token = env["TOKEN"].clone();
    let devices = tuwunel.state().tokens.len();
    let device = m.store.bots().load(&id).unwrap().bots["default"]
        .device_id
        .clone();

    let parse = |compose: &str| PluginPackage::parse(compose, &super::langs()).unwrap();
    upgrade_expect_rollback(&m, &docker, &id, &bot, &token).await;

    let v2 = echo("1.2.0", "");
    m.upgrade_upload(
        &id,
        parse(&v2),
        permissions(&v2),
        Vec::new(),
        false,
        &super::langs(),
    )
    .await
    .unwrap();
    let env = env_of_container(&docker, &bot);
    assert_eq!(env["TOKEN"], token);
    assert_eq!(env["AUTH"], format!("Bearer {token}"));
    assert_eq!(env["HTTP_PROXY"], "http://egress.wonton.internal:3128");
    assert_eq!(image(&docker, &bot), "echo:1.2.0");
    assert_eq!(tuwunel.state().tokens.len(), devices, "没有签新设备");
    assert_eq!(
        m.store.bots().load(&id).unwrap().bots["default"].device_id,
        device
    );
    assert!(m.store.load(&id).unwrap().unwrap().proxied);
    assert_eq!(m.status_of(&id), Some(PluginStatus::Running));

    // 令牌读不回来时报错，不悄悄签新设备。
    m.engine.remove_container(&bot).await.unwrap();
    let v3 = echo("1.3.0", "");
    let err = m
        .upgrade_upload(
            &id,
            parse(&v3),
            permissions(&v3),
            Vec::new(),
            false,
            &super::langs(),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(&super::message(
            "plugin-settings-egress-token-read-back",
            &[
                (
                    "reason",
                    &super::message(
                        "plugin-settings-egress-old-container-gone",
                        &[("container", "wonton-app-echo--bot")],
                    )
                ),
                (
                    "reinstall",
                    &super::message("plugin-settings-egress-token-reinstall", &[]),
                ),
            ],
        )),
        "{err}"
    );
    assert_eq!(tuwunel.state().tokens.len(), devices);
    assert_eq!(m.status_of(&id), Some(PluginStatus::Running));
}
