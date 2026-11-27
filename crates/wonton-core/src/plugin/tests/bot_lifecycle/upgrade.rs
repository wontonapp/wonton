//! 升级：network_mode 的 Bot 组升级保留身份，卸载删掉整组。

use super::{Bed, echo, granted, id, service_container};
use crate::plugin::UpgradeSwitches;

#[tokio::test]
async fn network_mode_bot_group_upgrade_preserves_its_identity_and_removes_the_whole_group() {
    let bed = Bed::start().await;
    let m = bed.manager();
    let source = echo()
        .replace(
            "x-wonton:",
            "  proxy:\n    image: busybox\n    network_mode: service:bot\nx-wonton:",
        )
        .replace(
            "  permissions: [matrix.bot]",
            "  bots: {services: [bot, proxy]}\n  permissions: [matrix.bot]",
        );
    m.install(&source, &granted(), super::super::langs())
        .await
        .unwrap();
    // bot 和 proxy 都是 Bot 服务：容器按 Bot 一份，proxy 共用同一个 Bot 那份 bot 容器的网络命名空间。
    let target = service_container("app-echo", "bot");
    let proxy = service_container("app-echo", "proxy");
    let env_map = |container: &str| {
        bed.docker
            .env(container)
            .iter()
            .filter_map(|e| e.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect::<std::collections::BTreeMap<String, String>>()
    };
    let before = env_map(&target);
    let package = super::super::super::PluginPackage::parse(
        &source.replace("version: 1.0.0", "version: 1.1.0"),
        &super::super::langs(),
    )
    .unwrap();
    m.upgrade_checked(
        package,
        &granted(),
        &UpgradeSwitches::Requested(Vec::new()),
        false,
        &[],
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(env_map(&target), before);
    assert!(bed.token_valid(&before["TOKEN"]));
    assert_eq!(
        bed.docker.created(&proxy).unwrap()["HostConfig"]["NetworkMode"],
        format!("container:{target}")
    );
    m.uninstall(&id(), false, &super::super::langs())
        .await
        .unwrap();
    assert!(!bed.docker.containers().contains(&target));
    assert!(!bed.docker.containers().contains(&proxy));
    assert!(!bed.token_valid(&before["TOKEN"]));
}
