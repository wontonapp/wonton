//! 核心重启后只起来一部分服务时的局域网端口：插件交回巡检，不卡在局域网异常上。地址变了时的端口搬移由
//! 核对的重建做（见 tests/lan.rs 的 `a_core_restarted_after_a_dhcp_change_moves_the_ports`）。

use std::sync::{Arc, Mutex};

use serde_json::json;

use super::super::fake_docker::FakeDocker;
use super::super::lan::{Detector, Lan};
use super::super::languages::Languages;
use super::super::names::container_name;
use super::super::{PluginId, PluginStatus};
use super::lan::{Host, add_default_bridge, granted, office, samba, status};
use super::manager::manager_with;

// 核心重启后 web 容器坏着（探针失败）：核对拿到部分部署，局域网异常清掉，插件交回巡检，
// 不卡在局域网异常上。web 好了以后巡检接手恢复。
#[tokio::test]
async fn a_partial_restart_hands_the_plugin_back_to_the_watch() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let detected: Host = Arc::new(Mutex::new(Ok(office())));
    let lan = || Lan::new(Detector::Fixed(detected.clone()), None);
    let compose = samba("");
    manager_with(engine.clone(), &dir)
        .with_lan(lan())
        .install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap();

    // 宿主重启：web 容器起不来（探针失败）。
    let web = container_name("app-test", "web");
    docker.set_state(
        &web,
        json!({
            "Status": "running", "Running": true,
            "Health": {"Status": "unhealthy", "Log": [{"ExitCode": 1, "Output": "probe failed"}]},
        }),
    );
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan()));
    m.restore().await.unwrap();
    let id = PluginId::parse("app.test").unwrap();
    assert!(m.lan.is_degraded(&id), "重启后要核一遍地址");

    // 核对拿到的是部分部署：局域网异常清掉，原因是容器。
    m.lan_pass(&Languages::default()).await.unwrap();
    assert!(!m.lan.is_degraded(&id));
    let PluginStatus::Degraded { reason } = status(&m) else {
        panic!("{:?}", status(&m));
    };
    assert!(reason.contains(&web), "原因是容器，不是局域网：{reason}");

    // web 好了：巡检接手，插件恢复运行。
    docker.set_state(&web, json!({"Status": "running", "Running": true}));
    m.watch_containers().await;
    assert_eq!(status(&m), PluginStatus::Running);
}
