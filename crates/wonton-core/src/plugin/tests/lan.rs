//! 局域网端口（`network.lan`，ADR-0019）：端口只发布在选出的地址上；不能出网的插件另接一个关掉伪装的网络；
//! 地址变了只重建开了局域网端口的容器；选不出地址时装不上、已装的标为异常，地址回来后恢复。

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use wonton_protocol::{LanChoice, LanFailure, LanInterfaceKind};

use super::super::fake_docker::FakeDocker;
use super::super::lan::probe::{DefaultRoute, HostAddress, HostInterface, HostNetwork};
use super::super::lan::{Detector, Lan};
use super::super::languages::Languages;
use super::super::names::container_name;
use super::super::package::PackageError;
use super::super::render::dry_run;
use super::super::{Permission, PluginId, PluginManager, PluginPackage, PluginStatus};
use super::manager::{manager_in_container, manager_with};
use super::{env_of_container, package};

pub(super) type Host = Arc<Mutex<Result<HostNetwork, String>>>;

fn nic(name: &str, addresses: &[&str]) -> HostInterface {
    HostInterface {
        name: name.into(),
        physical: true,
        addresses: addresses
            .iter()
            .map(|a| HostAddress {
                ip: a.parse().unwrap(),
                secondary: false,
                prefix: 24,
            })
            .collect(),
        ..Default::default()
    }
}

/// 有线 eno1 走默认路由，另有 Wi-Fi、Tailscale 和 Docker 的默认网桥。
fn host(eno1: &[&str]) -> HostNetwork {
    let mut wifi = nic("wlp2s0", &["192.168.0.82"]);
    wifi.devtype = Some("wlan".into());
    let mut tailscale = nic("tailscale0", &["100.101.102.103"]);
    tailscale.physical = false;
    tailscale.tun = true;
    let mut docker0 = nic("docker0", &["172.17.0.1"]);
    docker0.devtype = Some("bridge".into());
    docker0.physical = false;
    HostNetwork {
        routes: vec![
            DefaultRoute {
                interface: "eno1".into(),
                metric: 100,
            },
            DefaultRoute {
                interface: "wlp2s0".into(),
                metric: 600,
            },
        ],
        interfaces: vec![nic("eno1", eno1), wifi, tailscale, docker0],
    }
}

pub(super) fn office() -> HostNetwork {
    host(&["192.168.0.193", "fd00::10", "2408:8207::1"])
}

/// Docker 的默认网桥：网卡名写在网络的选项里。
pub(super) fn add_default_bridge(docker: &FakeDocker) {
    let options = json!({"Options": {"com.docker.network.bridge.name": "docker0"}});
    docker.add_network("bridge", "", options);
}

pub(super) async fn on(
    network: HostNetwork,
) -> (FakeDocker, Arc<PluginManager>, Host, tempfile::TempDir) {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let detected: Host = Arc::new(Mutex::new(Ok(network)));
    let lan = Lan::new(Detector::Fixed(detected.clone()), None);
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan));
    (docker, m, detected, dir)
}

/// Samba 式的插件：`samba` 开 445/tcp 和 137/udp，地址经变量交给它；`web` 不开局域网端口。
fn samba(extra_permissions: &str) -> String {
    package(
        "  samba:\n    image: example/samba\n    environment: {ADDRESSES: '${WONTON_LAN_ADDRESSES}'}\n  \
         web:\n    image: nginx",
        &format!(
            "  lan:\n    ports:\n      - {{service: samba, port: 445}}\n      \
             - {{service: samba, port: 137, protocol: udp}}\n  \
             permissions: [network.lan{extra_permissions}]"
        ),
    )
}

pub(super) fn granted(compose: &str) -> Vec<Permission> {
    PluginPackage::parse(compose).unwrap().manifest.permissions
}

pub(super) fn created(docker: &FakeDocker, service: &str) -> Value {
    docker
        .created(&container_name("app-test", service))
        .expect("容器已创建")
}

pub(super) fn binding(ip: &str, port: &str) -> Value {
    json!([{"HostIp": ip, "HostPort": port}])
}

fn status(m: &PluginManager) -> PluginStatus {
    m.list().into_iter().next().expect("装了插件").status
}

#[tokio::test]
async fn ports_are_published_only_on_the_chosen_address() {
    let (docker, m, _, _dir) = on(office()).await;
    let compose = samba("");
    m.install(&compose, &granted(&compose)).await.unwrap();

    let samba = created(&docker, "samba");
    let bindings = &samba["HostConfig"]["PortBindings"];
    assert_eq!(bindings["445/tcp"], binding("192.168.0.193", "445"));
    assert_eq!(bindings["137/udp"], binding("192.168.0.193", "137"));
    assert_eq!(samba["ExposedPorts"], json!({"445/tcp": {}, "137/udp": {}}));
    let env = env_of_container(&docker, "wonton-app-test--samba");
    assert_eq!(env["ADDRESSES"], "192.168.0.193");
    // 插件网络是内部网络，Docker 不在上面发布端口：samba 另接一个关掉伪装的网络，默认路由也走它。
    let lan = docker
        .network("wonton-app-test--lan")
        .expect("建了局域网的网络");
    assert_eq!(lan["Internal"], json!(false));
    assert_eq!(
        lan["Options"]["com.docker.network.bridge.enable_ip_masquerade"],
        "false"
    );
    let endpoints = &samba["NetworkingConfig"]["EndpointsConfig"];
    assert_eq!(endpoints["wonton-app-test--lan"]["GwPriority"], json!(1));
    assert!(endpoints["wonton-app-test"].is_object(), "仍在插件网络里");
    // 没列在 lan.ports 里的服务什么都不开。
    let web = created(&docker, "web");
    assert_eq!(web["HostConfig"]["PortBindings"], Value::Null);
    let web_endpoints = web["NetworkingConfig"]["EndpointsConfig"]
        .as_object()
        .unwrap();
    assert_eq!(web_endpoints.len(), 1);

    // 卸载时连同局域网的网络一起删。
    let id = PluginId::parse("app.test").unwrap();
    m.uninstall(&id, false).await.unwrap();
    assert_eq!(docker.networks(), ["bridge"]);
}

#[tokio::test]
async fn egress_plugins_publish_on_their_own_network() {
    let (docker, m, _, _dir) = on(office()).await;
    let compose = samba(", network.egress");
    m.install(&compose, &granted(&compose)).await.unwrap();
    assert_eq!(docker.network("wonton-app-test--lan"), None);
    let samba = created(&docker, "samba");
    let endpoints = samba["NetworkingConfig"]["EndpointsConfig"]
        .as_object()
        .unwrap();
    assert_eq!(endpoints.len(), 1);
    let bindings = &samba["HostConfig"]["PortBindings"];
    assert_eq!(bindings["445/tcp"], binding("192.168.0.193", "445"));
}

/// 和插件网络一样：上次没装完留下的清掉，主人建的不动。
#[tokio::test]
async fn a_leftover_lan_network_is_replaced_but_one_the_owner_created_is_refused() {
    const LAN: &str = "wonton-app-test--lan";
    let (docker, m, _, _dir) = on(office()).await;
    docker.add_network(LAN, "app.test", json!({}));
    let compose = samba("");
    m.install(&compose, &granted(&compose)).await.unwrap();
    assert_eq!(docker.network(LAN).unwrap()["Name"], json!(LAN));

    m.uninstall(&PluginId::parse("app.test").unwrap(), false)
        .await
        .unwrap();
    docker.add_unlabeled_network(LAN);
    let err = m.install(&compose, &granted(&compose)).await.unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
    assert!(err.to_string().contains(LAN), "{err}");
    assert_eq!(docker.network(LAN), Some(json!({})));
    assert_eq!(docker.network("wonton-app-test"), None);
}

#[tokio::test]
async fn previews_show_each_port_and_why_it_cannot_be_installed() {
    let (_docker, m, detected, _dir) = on(office()).await;
    let package = PluginPackage::parse(&samba("")).unwrap();
    let preview = m
        .preview_install(&package, &Languages::default())
        .await
        .unwrap();
    assert_eq!(preview.blocked, None);
    let lines: Vec<&str> = preview
        .notices
        .iter()
        .filter(|n| n.emphasis)
        .map(|n| n.text.as_str())
        .collect();
    assert!(lines.contains(&"局域网端口：samba 的 445/tcp，开在 eno1 的 192.168.0.193 上"));
    assert!(lines.contains(&"局域网端口：samba 的 137/udp，开在 eno1 的 192.168.0.193 上"));
    assert!(
        lines.iter().any(|l| l.contains("不经核心登录")),
        "{lines:?}"
    );

    // 默认路由走 Tailscale：不自动选，装不上，原因写清。
    let mut tunnel = office();
    tunnel.routes[0].interface = "tailscale0".into();
    *detected.lock().unwrap() = Ok(tunnel);
    let preview = m
        .preview_install(&package, &Languages::default())
        .await
        .unwrap();
    let blocked = preview.blocked.expect("装不上");
    assert!(blocked.contains("tailscale0"), "{blocked}");
    let compose = samba("");
    let err = m.install(&compose, &granted(&compose)).await.unwrap_err();
    assert_eq!(err.kind(), "lan_unavailable", "{err}");
    assert!(m.list().is_empty());

    // 检测容器起不来也一样。
    *detected.lock().unwrap() = Err("没有权限".into());
    let err = m.install(&compose, &granted(&compose)).await.unwrap_err();
    assert_eq!(err.kind(), "lan_unavailable", "{err}");
    assert!(err.to_string().contains("没有权限"), "{err}");
}

#[tokio::test]
async fn ports_taken_by_another_plugin_are_refused_before_anything_starts() {
    let (docker, m, _, _dir) = on(office()).await;
    let first = samba("");
    m.install(&first, &granted(&first)).await.unwrap();
    let second = samba("").replace("id: app.test", "id: app.other");
    let package = PluginPackage::parse(&second).unwrap();
    let preview = m
        .preview_install(&package, &Languages::default())
        .await
        .unwrap();
    let blocked = preview.blocked.expect("端口冲突");
    assert!(
        blocked.contains("445/tcp") && blocked.contains("app.test"),
        "{blocked}"
    );
    let before = docker.writes().len();
    let err = m.install(&second, &granted(&second)).await.unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
    assert_eq!(docker.writes().len(), before, "冲突时什么也不建");
}

#[tokio::test]
async fn an_address_change_rebuilds_only_the_lan_containers() {
    let (docker, m, detected, _dir) = on(office()).await;
    let compose = samba("");
    m.install(&compose, &granted(&compose)).await.unwrap();
    // 地址没变时什么也不动。
    let before = docker.writes().len();
    m.lan_pass().await.unwrap();
    assert_eq!(docker.writes().len(), before);

    // DHCP 换了地址。
    *detected.lock().unwrap() = Ok(host(&["192.168.0.68"]));
    m.lan_pass().await.unwrap();
    let samba = created(&docker, "samba");
    let bindings = &samba["HostConfig"]["PortBindings"];
    assert_eq!(bindings["445/tcp"], binding("192.168.0.68", "445"));
    let env = env_of_container(&docker, "wonton-app-test--samba");
    assert_eq!(env["ADDRESSES"], "192.168.0.68");
    let writes = docker.writes()[before..].to_vec();
    assert!(writes.contains(&"DELETE /containers/wonton-app-test--samba".to_owned()));
    assert!(
        !writes.iter().any(|w| w.contains("wonton-app-test--web")),
        "别的容器不动：{writes:?}"
    );
    assert!(
        !writes
            .iter()
            .any(|w| w.starts_with("POST /networks/create")),
        "网络不重建：{writes:?}"
    );
    assert_eq!(status(&m), PluginStatus::Running);
}

#[tokio::test]
async fn a_rebuild_that_needs_a_missing_email_leaves_the_old_container_alone() {
    let (docker, mut m, detected, dir) = on(office()).await;
    let compose = package(
        "  samba:\n    image: example/samba\n    environment: {MAIL: '${WONTON_OWNER_EMAIL:?}'}",
        "  lan:\n    ports:\n      - {service: samba, port: 445}\n  \
         permissions: [network.lan, owner.identity]",
    );
    m.install(&compose, &granted(&compose)).await.unwrap();
    let state = dir.path().join("owner-without-profile");
    crate::owner::lock_username(&state, None).unwrap();
    Arc::get_mut(&mut m).unwrap().cfg.profile = crate::owner::OwnerStore::open(&state).unwrap();

    let before = docker.writes().len();
    *detected.lock().unwrap() = Ok(host(&["192.168.0.68"]));
    m.lan_pass().await.unwrap();
    assert_eq!(docker.writes().len(), before, "旧容器不动");
    let PluginStatus::Degraded { reason } = status(&m) else {
        panic!("应标为异常：{:?}", status(&m));
    };
    assert!(reason.contains(crate::owner::NO_EMAIL), "{reason}");
}

#[tokio::test]
async fn a_lost_address_degrades_the_plugin_until_it_comes_back() {
    let (docker, m, detected, _dir) = on(office()).await;
    let compose = samba("");
    m.install(&compose, &granted(&compose)).await.unwrap();
    let mut unplugged = office();
    unplugged.routes.clear();
    *detected.lock().unwrap() = Ok(unplugged);
    m.lan_pass().await.unwrap();
    let PluginStatus::Degraded { reason } = status(&m) else {
        panic!("应标为异常：{:?}", status(&m));
    };
    assert!(reason.contains("默认路由"), "{reason}");
    let before = docker.writes().len();
    m.lan_pass().await.unwrap();
    assert_eq!(docker.writes().len(), before, "端口不改开到别的地址上");

    // 地址回来了，发布的地址没变：不重建，核对后恢复。
    *detected.lock().unwrap() = Ok(office());
    m.lan_pass().await.unwrap();
    assert_eq!(status(&m), PluginStatus::Running);
    assert!(
        !docker.writes()[before..]
            .iter()
            .any(|w| w.starts_with("DELETE")),
        "{:?}",
        docker.writes()
    );
}

#[tokio::test]
async fn a_core_restarted_after_a_dhcp_change_moves_the_ports() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let detected: Host = Arc::new(Mutex::new(Ok(office())));
    let lan = || Lan::new(Detector::Fixed(detected.clone()), None);
    let first = manager_with(engine.clone(), &dir).with_lan(lan());
    let compose = samba("");
    first.install(&compose, &granted(&compose)).await.unwrap();
    // 宿主重启，地址变了：容器发布不到旧地址上，起不来（Docker 删不掉它们之前，这里直接让它不在）。
    engine
        .remove_container("wonton-app-test--samba")
        .await
        .unwrap();
    *detected.lock().unwrap() = Ok(host(&["192.168.0.68"]));
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan()));
    m.restore().await.unwrap();
    assert!(matches!(status(&m), PluginStatus::Degraded { .. }));
    m.lan_pass().await.unwrap();
    assert_eq!(status(&m), PluginStatus::Running);
    let bindings = &created(&docker, "samba")["HostConfig"]["PortBindings"];
    assert_eq!(bindings["445/tcp"], binding("192.168.0.68", "445"));
}

#[tokio::test]
async fn settings_list_interfaces_and_check_new_choices() {
    let (docker, m, _, _dir) = on(office()).await;
    let compose = samba("");
    m.install(&compose, &granted(&compose)).await.unwrap();
    let langs = Languages::default();
    let settings = m.lan_settings(&langs).await.unwrap();
    assert_eq!(settings.choice, LanChoice::Auto);
    assert!(!settings.fixed_by_environment);
    assert_eq!(settings.addresses.len(), 1);
    assert_eq!(settings.addresses[0].address, "192.168.0.193");
    assert_eq!(settings.addresses[0].interface.as_deref(), Some("eno1"));
    // Docker 的网桥按 Docker 的网络列表认出来，不进候选。
    let kinds: Vec<(&str, LanInterfaceKind)> = settings
        .interfaces
        .iter()
        .map(|i| (i.name.as_str(), i.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("eno1", LanInterfaceKind::Ethernet),
            ("tailscale0", LanInterfaceKind::Tunnel),
            ("wlp2s0", LanInterfaceKind::Wifi)
        ]
    );
    let eno1: Vec<(&str, bool)> = settings.interfaces[0]
        .addresses
        .iter()
        .map(|a| (a.address.as_str(), a.proxied))
        .collect();
    assert_eq!(eno1, [("192.168.0.193", false), ("fd00::10", true)]);
    assert_eq!(settings.plugins.len(), 1);
    assert_eq!(settings.plugins[0].ports.len(), 2);

    let manual = |interfaces: &[&str], addresses: &[&str]| LanChoice::Manual {
        interfaces: interfaces.iter().map(|s| (*s).to_owned()).collect(),
        addresses: addresses.iter().map(|s| (*s).to_owned()).collect(),
    };
    for (choice, kind) in [
        (manual(&[], &[]), "invalid_input"),
        (manual(&["docker0"], &[]), "invalid_input"),
        (manual(&[], &["2408:8207::1"]), "invalid_input"),
        (LanChoice::Unknown, "invalid_input"),
    ] {
        let err = m.set_lan_choice(choice, langs.clone()).await.unwrap_err();
        assert_eq!(err.kind(), kind, "{err}");
    }
    // 加一个 IPv6 地址：选择存下，插件按新地址重建，做完再返回。
    let choice = manual(&["eno1"], &["fd00::10"]);
    let settings = m
        .set_lan_choice(choice.clone(), langs.clone())
        .await
        .unwrap();
    assert_eq!(settings.choice, choice);
    let addresses: Vec<(&str, bool)> = settings
        .addresses
        .iter()
        .map(|a| (a.address.as_str(), a.proxied))
        .collect();
    assert_eq!(addresses, [("192.168.0.193", false), ("fd00::10", true)]);
    let bindings = &created(&docker, "samba")["HostConfig"]["PortBindings"];
    assert_eq!(
        bindings["445/tcp"],
        json!([
            {"HostIp": "192.168.0.193", "HostPort": "445"},
            {"HostIp": "fd00::10", "HostPort": "445"}
        ])
    );
    assert_eq!(
        m.lan_settings(&langs).await.unwrap().choice,
        choice,
        "选择存下了"
    );
}

#[tokio::test]
async fn addresses_fixed_by_the_environment_cannot_be_changed() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let detected: Host = Arc::new(Mutex::new(Err("不该用到".into())));
    let fixed = vec!["192.168.0.193".parse().unwrap()];
    let lan = Lan::new(Detector::Fixed(detected), Some(fixed));
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan));
    // 检测不了网卡时照样用指定的地址。
    let settings = m.lan_settings(&Languages::default()).await.unwrap();
    assert!(settings.fixed_by_environment);
    assert_eq!(settings.addresses[0].address, "192.168.0.193");
    assert_eq!(settings.addresses[0].interface, None);
    let err = m
        .set_lan_choice(LanChoice::Auto, Languages::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
}

#[tokio::test]
async fn the_probe_runs_unprivileged_in_the_host_network_and_is_always_removed() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    docker.add_image_container("wonton-core", "sha256:core");
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_in_container(engine, &dir));
    let probe = "wonton-lan-probe";
    docker.set_output(probe, 0, &serde_json::to_string(&office()).unwrap(), "");
    let settings = m.lan_settings(&Languages::default()).await.unwrap();
    assert_eq!(settings.addresses[0].address, "192.168.0.193");
    let body = docker.created(probe).unwrap();
    assert_eq!(body["Image"], "sha256:core");
    assert_eq!(body["Cmd"], json!(["lan-probe"]));
    assert_eq!(body["User"], "65534:65534");
    let host = &body["HostConfig"];
    assert_eq!(host["NetworkMode"], "host");
    assert_eq!(host["CapDrop"], json!(["ALL"]));
    assert_eq!(host["ReadonlyRootfs"], json!(true));
    assert_eq!(host["SecurityOpt"], json!(["no-new-privileges"]));
    assert_eq!(host["Mounts"], Value::Null);
    assert!(!docker.containers().contains(&probe.to_owned()), "跑完就删");

    docker.set_output(probe, 1, "", "读 /proc/net/route 失败");
    let settings = m.lan_settings(&Languages::default()).await.unwrap();
    let problem = settings.problem.expect("检测失败");
    assert_eq!(problem.reason, LanFailure::DetectionFailed);
    assert!(
        problem.message.contains("/proc/net/route"),
        "{}",
        problem.message
    );
    assert!(
        !docker.containers().contains(&probe.to_owned()),
        "失败了也删"
    );
}

#[test]
fn manifests_must_declare_ports_and_permission_together() {
    let parse = |extra: &str| {
        let compose = package("  samba:\n    image: example/samba", extra);
        PluginPackage::parse(&compose).map(|p| dry_run(&p).map(|_| ()))
    };
    let invalid = |extra: &str| match parse(extra) {
        Err(PackageError::Manifest(e)) => e.to_string(),
        other => panic!("{extra} 应不合法：{other:?}"),
    };
    let port = "  lan: {ports: [{service: samba, port: 445}]}\n";
    assert!(invalid(&format!("{port}  permissions: []")).contains("network.lan"));
    assert!(invalid("  permissions: [network.lan]").contains("lan.ports"));
    let bot = format!("{port}  bot: {{}}\n  permissions: [network.lan, matrix.bot]");
    assert!(invalid(&bot).contains("matrix.bot"));
    let twice = "  lan: {ports: [{service: samba, port: 445}, {service: samba, port: 445}]}\n  \
                 permissions: [network.lan]";
    assert!(invalid(twice).contains("重复"));
    let zero = "  lan: {ports: [{service: samba, port: 0}]}\n  permissions: [network.lan]";
    assert!(invalid(zero).contains("0"));
    let web = "  mcp: {service: samba, port: 80, transport: streamable-http}\n  lan: {ports: [{service: samba, port: 80}]}\n  \
               permissions: [network.lan]";
    assert!(invalid(web).contains("mcp"));
    // 同一个端口号的 UDP 不是 MCP。
    let udp = web.replace("port: 80}]", "port: 80, protocol: udp}]");
    assert!(matches!(parse(&udp), Ok(Ok(()))));
    // 网页的端口可以列，默认不开（`lan_webui`）。
    let webui = "  webui: {service: samba, port: 80}\n  lan: {ports: [{service: samba, port: 80}]}\n  \
                 permissions: [network.lan]";
    assert!(matches!(parse(webui), Ok(Ok(()))));
    let unknown = "  lan: {ports: [{service: nas, port: 445}]}\n  permissions: [network.lan]";
    assert!(matches!(
        parse(unknown),
        Err(PackageError::UnknownService { .. })
    ));
    let isolated = package(
        "  samba:\n    image: example/samba\n    network_mode: none",
        &format!("{port}  permissions: [network.lan]"),
    );
    let pkg = PluginPackage::parse(&isolated).unwrap();
    let err = dry_run(&pkg).unwrap_err().to_string();
    assert!(err.contains("network_mode: none"), "{err}");
}

#[test]
fn the_example_is_valid() {
    let pkg = PluginPackage::parse(include_str!("examples/lan.yaml")).unwrap();
    let ports = &pkg.manifest.lan.as_ref().expect("有 lan").ports;
    assert_eq!(ports[0].key(), "445/tcp");
    dry_run(&pkg).unwrap();
}
