//! 核心重启：安装半路重启留下的设备、重启后设备被吊销的 Bot。

use std::sync::Arc;

use super::{Bed, echo, granted};
use crate::plugin::PluginStatus;
use crate::plugin::names::container_name;

/// 核心在安装半路重启：上次登录的设备没人吊销，它的令牌还在残留的容器里。再装时吊销，只留这次的一台。
#[tokio::test]
async fn a_core_restart_midway_through_an_install_revokes_the_device_it_left_behind() {
    let bed = Bed::start().await;
    let bot = container_name("app-echo", "bot");
    let held = bed.docker.hold(&format!("POST /containers/{bot}/start"));
    let m = Arc::new(bed.manager());
    let task = tokio::spawn({
        let m = m.clone();
        async move { m.install(&echo(), &granted(), super::super::langs()).await }
    });
    // 安装要是先失败就没人在等这个请求；超时把安装任务的状态和错误一并带出来，别只剩一句超时。
    if tokio::time::timeout(std::time::Duration::from_secs(5), held.reached())
        .await
        .is_err()
    {
        let install = if task.is_finished() {
            match task.await {
                Ok(Ok(_)) => "安装已经完成".to_owned(),
                Ok(Err(e)) => format!("安装先失败了：{e}"),
                Err(e) => format!("安装任务结束了：{e}"),
            }
        } else {
            "安装还在跑".to_owned()
        };
        panic!("5 秒内没有等到假 Docker 收到 POST /containers/{bot}/start，{install}");
    }
    task.abort();
    task.await.unwrap_err();
    held.release();
    let orphan = bed.env();
    assert!(bed.token_valid(&orphan["TOKEN"]));

    bed.manager()
        .install(&echo(), &granted(), super::super::langs())
        .await
        .unwrap();
    let env = bed.env();
    assert!(!bed.token_valid(&orphan["TOKEN"]), "上次的令牌不能还能用");
    assert!(bed.token_valid(&env["TOKEN"]));
    assert_eq!(env["ROOM"], orphan["ROOM"], "账号和主房间沿用");
    assert_eq!(bed.tuwunel.state().devices("@echo:test").len(), 1);
}

#[tokio::test]
async fn a_restart_reports_a_bot_whose_device_was_revoked() {
    let bed = Bed::start().await;
    bed.manager()
        .install(&echo(), &granted(), super::super::langs())
        .await
        .unwrap();
    let restarted = bed.manager();
    restarted.restore().await.unwrap();
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);

    // 设备被吊销了（比如主人在别的客户端里删了它）：插件拿着的令牌已经失效，如实标为异常。
    let token = bed.env()["TOKEN"].clone();
    bed.tuwunel.state().tokens.remove(&token);
    let restarted = bed.manager();
    restarted.restore().await.unwrap();
    let PluginStatus::Degraded { reason } = &restarted.list()[0].status else {
        panic!("设备被吊销的 Bot 插件应标为异常");
    };
    assert!(
        reason.contains(&super::super::message(
            "plugin-settings-bot-device-revoked",
            &[("user", "@echo:test")],
        )),
        "{reason}"
    );
    // 状态异常也照样认得出来源：插件恢复正常前也能经核心连 Matrix。
    assert!(restarted.source(bed.source_address()).is_some());
}
