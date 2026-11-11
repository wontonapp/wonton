//! 启动器声明的宣告地址（`WONTON_ANNOUNCE_ADDRESSES`）和桌面虚拟机（ADR-0019）：宣告地址只用来告诉手机和插件，
//! 端口绑通配地址；认出桌面虚拟机又没声明时如实报错。

use std::sync::{Arc, Mutex};

use serde_json::json;
use wonton_protocol::{LanChoice, LanFailure};

use super::super::PluginManager;
use super::super::fake_docker::FakeDocker;
use super::super::lan::probe::{DefaultRoute, HostAddress, HostInterface, HostNetwork};
use super::super::lan::{Detector, Lan};
use super::super::languages::Languages;
use super::env_of_container;
use super::lan::{Host, add_default_bridge, created, granted, office, samba};
use super::manager::manager_with;

/// 同 `lan::on`，但启动器声明了宣告地址。
pub(super) async fn on_announced(
    network: HostNetwork,
    announce: &str,
) -> (FakeDocker, Arc<PluginManager>, Host, tempfile::TempDir) {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let detected: Host = Arc::new(Mutex::new(Ok(network)));
    let lan = Lan::new(Detector::Fixed(detected.clone()), None)
        .with_announce(super::super::lan::announce::parse(announce).unwrap());
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan));
    (docker, m, detected, dir)
}

/// 桌面虚拟机：内核串认得出，网卡地址是虚拟机 NAT 的。
fn orbstack() -> HostNetwork {
    let mut wifi = HostInterface {
        name: "eth0".into(),
        physical: true,
        addresses: vec![HostAddress {
            ip: "192.168.0.5".parse().unwrap(),
            secondary: false,
            prefix: 24,
        }],
        ..Default::default()
    };
    wifi.physical = true;
    HostNetwork {
        kernel: Some("6.12.4-orbstack-00286-g339578031cbf".into()),
        routes: vec![DefaultRoute {
            interface: "eth0".into(),
            metric: 100,
        }],
        interfaces: vec![wifi],
        subnets: vec![],
        dmi: None,
    }
}

#[tokio::test]
async fn announced_addresses_are_bound_on_the_wildcard_and_told_to_phones() {
    let (docker, m, _, _dir) = on_announced(office(), "192.168.0.5/24, fd00::5").await;
    let compose = samba("");
    m.install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap();
    // 宣告地址不在虚拟机的网卡上，绑通配地址；单元里 IPv6 是 docker-proxy，绑 :: 一起。
    let bindings = &created(&docker, "samba")["HostConfig"]["PortBindings"];
    assert_eq!(
        bindings["445/tcp"],
        json!([
            {"HostIp": "0.0.0.0", "HostPort": "445"},
            {"HostIp": "::", "HostPort": "445"}
        ])
    );
    // 插件拿到的是宣告地址，不是通配地址。
    let env = env_of_container(&docker, "wonton-app-test--samba");
    assert_eq!(env["ADDRESSES"], "192.168.0.5,fd00::5");
}

#[tokio::test]
async fn announced_settings_show_both_and_cannot_be_chosen_away() {
    let (_docker, m, _, _dir) = on_announced(office(), "192.168.0.5").await;
    let settings = m.lan_settings(&Languages::default()).await.unwrap();
    assert!(settings.fixed_by_environment);
    assert!(settings.announce_by_environment);
    assert_eq!(settings.addresses[0].address, "0.0.0.0");
    assert_eq!(settings.announce_addresses[0].address, "192.168.0.5");
    assert_eq!(settings.problem, None);
    let err = m
        .set_lan_choice(LanChoice::Auto, Languages::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
}

#[tokio::test]
async fn announced_addresses_work_when_the_detection_fails() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let detected: Host = Arc::new(Mutex::new(Err("检测容器起不来".into())));
    let lan = Lan::new(Detector::Fixed(detected), None)
        .with_announce(super::super::lan::announce::parse("192.168.0.5").unwrap());
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan));
    let settings = m.lan_settings(&Languages::default()).await.unwrap();
    assert_eq!(settings.addresses[0].address, "0.0.0.0");
    assert_eq!(settings.announce_addresses[0].address, "192.168.0.5");
    assert_eq!(settings.problem, None, "宣告地址不靠检测");
}

#[tokio::test]
async fn desktop_vms_report_themselves_without_an_announced_address() {
    let (_docker, m, _, _dir) = on_announced(orbstack(), "192.168.0.5").await;
    let settings = m.lan_settings(&Languages::default()).await.unwrap();
    assert_eq!(settings.problem, None, "声明了宣告地址就不报");

    // 没声明时照样认出桌面虚拟机并报错。
    let (_docker, m, _, _dir) = on_without_announce().await;
    let settings = m.lan_settings(&Languages::default()).await.unwrap();
    let problem = settings.problem.expect("报错");
    assert_eq!(problem.reason, LanFailure::DesktopVm);
    assert!(problem.message.contains("orbstack"), "{}", problem.message);
    let compose = samba("");
    let err = m
        .install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "lan_unavailable", "{err}");

    // 设了 WONTON_LAN_ADDRESSES 也照报：它写的是绑定地址，改不了手机连不到。
    let fixed = vec!["192.168.0.5".parse().unwrap()];
    let (_docker, m, _, _dir) = on_fixed(Some(fixed)).await;
    let problem = m
        .lan_settings(&Languages::default())
        .await
        .unwrap()
        .problem
        .expect("照样报错");
    assert_eq!(problem.reason, LanFailure::DesktopVm);
}

async fn on_without_announce() -> (FakeDocker, Arc<PluginManager>, Host, tempfile::TempDir) {
    on_fixed(None).await
}

/// 没有宣告地址；`fixed` 是 `WONTON_LAN_ADDRESSES`。
async fn on_fixed(
    fixed: Option<Vec<std::net::IpAddr>>,
) -> (FakeDocker, Arc<PluginManager>, Host, tempfile::TempDir) {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let detected: Host = Arc::new(Mutex::new(Ok(orbstack())));
    let lan = Lan::new(Detector::Fixed(detected.clone()), fixed);
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan));
    (docker, m, detected, dir)
}
