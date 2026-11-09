//! 插件网络的地址段：装很多个插件不撞池、不重叠；重建容器时把旧网段迁进地址段，卷和数据不动。

use std::net::Ipv4Addr;
use std::sync::Arc;

use ipnet::Ipv4Net;
use serde_json::json;

use super::super::core_address::last_usable;
use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::names::Instance;
use super::super::network_pool::NetworkPool;
use super::super::{PluginPackage, PluginStatus};
use super::lan::{granted, host, office, on};
use super::manager::manager_in_container;

/// 连装 40 个插件：每个网络都在配置的地址段里、两两不重叠，核心都留在各段最后一个可用地址上。
/// 40 已超过 Docker 默认地址池能分的网络数（约 30 个，#120）。
#[tokio::test]
async fn forty_plugins_all_land_in_the_pool_disjoint_with_the_core_at_the_last_address() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_in_container(engine, &dir));
    for i in 0..40 {
        let compose = format!(
            "services:\n  web:\n    image: example/web\nx-wonton:\n  spec: '0.5'\n  id: app.echo{i}\n  \
             version: 1.0.0\n  kind: app\n  title: {{zh: 回声{i}, en: Echo{i}}}\n  \
             description: {{zh: 测试, en: Test}}\n  permissions: []"
        );
        m.install(&compose, &[], super::langs()).await.unwrap();
    }
    let pool = NetworkPool::default();
    let mut subnets = Vec::new();
    for i in 0..40 {
        let network = Instance::default().network(&format!("app-echo{i}"));
        let body = docker
            .network(&network)
            .unwrap_or_else(|| panic!("网络 {network} 不存在"));
        let subnet: Ipv4Net = body["IPAM"]["Config"][0]["Subnet"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(subnet.prefix_len(), 26, "{network}: {subnet}");
        assert!(pool.covers(subnet), "{network}: {subnet} 不在地址段里");
        let address: Ipv4Addr =
            docker.endpoint(&network, "wonton-core").unwrap()["IPAMConfig"]["IPv4Address"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
        assert_eq!(
            last_usable(subnet),
            Some(address),
            "{network}: 核心不在 {subnet} 的最后一个可用地址上"
        );
        subnets.push(subnet);
    }
    for (i, left) in subnets.iter().enumerate() {
        for right in &subnets[i + 1..] {
            assert!(
                !left.contains(&right.network()) && !right.contains(&left.network()),
                "{left} 和 {right} 重叠"
            );
        }
    }
}

/// 旧版核心让 Docker 从默认池分的网段：重建容器时删掉在段内重建，卷和数据不动，插件回到运行。
#[tokio::test]
async fn rebuilding_migrates_an_old_network_into_the_pool_and_keeps_data() {
    let (docker, m, detected, _dir) = on(office()).await;
    let compose = super::package(
        "  samba:\n    image: example/samba\n    \
             environment: {ADDRESSES: '${WONTON_LAN_ADDRESSES}'}\n    \
             volumes: [data:/var/lib/samba]\n  web:\n    image: nginx\n\
             volumes:\n  data: {}",
        "  lan:\n    ports:\n      - {service: samba, port: 445}\n  \
             permissions: [network.lan]",
    );
    let package = PluginPackage::parse(&compose, &super::langs()).unwrap();
    let id = package.manifest.id.clone();
    let network = Instance::default().network(&id.slug());
    m.install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap();

    // 旧版核心留下的网络：Docker 从默认池分的 /16。
    docker.add_network(
        &network,
        id.as_str(),
        json!({
            "Internal": true,
            "IPAM": {"Config": [{"Subnet": "172.18.0.0/16", "Gateway": "172.18.0.1"}]}
        }),
    );

    *detected.lock().unwrap() = Ok(host(&["192.168.0.14"]));
    m.lan_pass(&Languages::default()).await.unwrap();

    let body = docker.network(&network).unwrap();
    let subnet: Ipv4Net = body["IPAM"]["Config"][0]["Subnet"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let pool = NetworkPool::default();
    assert!(pool.covers(subnet), "迁移后 {subnet} 不在地址段里");
    let volume = Instance::default().volume(&id.slug(), "data");
    assert!(
        docker.volumes().iter().any(|name| name == &volume),
        "数据卷 {volume} 应该还在"
    );
    let view = m
        .list()
        .into_iter()
        .find(|view| view.id == id)
        .expect("插件已装");
    assert!(
        matches!(view.status, PluginStatus::Running),
        "{:?}",
        view.status
    );
}

/// 旧版核心留下的三种网络——主网络、局域网端口的网络、出口容器和它的网络（整套核心共用）——
/// 在同一次重建里全部迁进地址段，卷和数据不动，插件回到运行。
#[tokio::test]
async fn rebuilding_migrates_main_lan_and_egress_networks_together() {
    let (docker, m, detected, _dir) = on(office()).await;
    let compose = super::package(
        "  samba:\n    image: example/samba\n    \
             environment: {ADDRESSES: '${WONTON_LAN_ADDRESSES}'}\n    \
             volumes: [data:/var/lib/samba]\n  web:\n    image: nginx\n\
             volumes:\n  data: {}",
        "  lan:\n    ports:\n      - {service: samba, port: 445}\n  \
             permissions: [network.lan, network.egress]",
    );
    let package = PluginPackage::parse(&compose, &super::langs()).unwrap();
    let id = package.manifest.id.clone();
    let network = Instance::default().network(&id.slug());
    let lan_network = Instance::default().lan_network(&id.slug());
    m.install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap();
    // 旧版核心留下的三个网络：Docker 从默认池分的 /16。出口网络整套核心只有一个。
    let old = |subnet: &str, gateway: &str| json!({"IPAM": {"Config": [{"Subnet": subnet, "Gateway": gateway}]}});
    docker.add_network(&network, id.as_str(), old("172.18.0.0/16", "172.18.0.1"));
    docker.add_network(
        &lan_network,
        id.as_str(),
        old("172.19.0.0/16", "172.19.0.1"),
    );
    docker.add_network("wonton-egress", "", old("172.20.0.0/16", "172.20.0.1"));

    *detected.lock().unwrap() = Ok(host(&["192.168.0.14"]));
    m.lan_pass(&Languages::default()).await.unwrap();

    let pool = NetworkPool::default();
    let mut subnets = Vec::new();
    for name in [&network, &lan_network, &"wonton-egress".to_owned()] {
        let body = docker
            .network(name)
            .unwrap_or_else(|| panic!("网络 {name} 不存在"));
        let subnet: Ipv4Net = body["IPAM"]["Config"][0]["Subnet"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(subnet.prefix_len(), 26, "{name}");
        assert!(pool.covers(subnet), "{name}: {subnet} 不在地址段里");
        subnets.push(subnet);
    }
    for (i, left) in subnets.iter().enumerate() {
        for right in &subnets[i + 1..] {
            assert!(
                !left.contains(&right.network()) && !right.contains(&left.network()),
                "{left} 和 {right} 重叠"
            );
        }
    }
    let volume = Instance::default().volume(&id.slug(), "data");
    assert!(
        docker.volumes().iter().any(|name| name == &volume),
        "数据卷 {volume} 应该还在"
    );
    let view = m
        .list()
        .into_iter()
        .find(|view| view.id == id)
        .expect("插件已装");
    assert!(
        matches!(view.status, PluginStatus::Running),
        "{:?}",
        view.status
    );
}

/// 渲染阶段失败：预留放掉，旧网络和旧容器一个不动、插件照原样跑；放掉的网段下一个插件能用上。
#[tokio::test]
async fn a_failed_render_releases_the_reservation_and_keeps_the_old_setup_running() {
    let (docker, m, detected, _dir) = on(office()).await;
    let compose = super::package(
        "  samba:\n    image: example/samba\n    \
             environment: {ADDRESSES: '${WONTON_LAN_ADDRESSES}'}\n    \
             volumes: [data:/var/lib/samba]\n  web:\n    image: nginx\n\
             volumes:\n  data: {}",
        "  lan:\n    ports:\n      - {service: samba, port: 445}\n  \
             permissions: [network.lan]",
    );
    let package = PluginPackage::parse(&compose, &super::langs()).unwrap();
    let id = package.manifest.id.clone();
    let network = Instance::default().network(&id.slug());
    let lan_network = Instance::default().lan_network(&id.slug());
    m.install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap();
    docker.add_network(
        &network,
        id.as_str(),
        json!({"Internal": true, "IPAM": {"Config": [{"Subnet": "172.18.0.0/16", "Gateway": "172.18.0.1"}]}}),
    );
    // 局域网网络弄丢：重建时要在渲染阶段重建它，注入的失败正好炸在那里。
    docker.remove_network(&lan_network);
    docker.reject_pinned_networks(1, "注入的建网失败");

    *detected.lock().unwrap() = Ok(host(&["192.168.0.14"]));
    m.lan_pass(&Languages::default()).await.unwrap();

    // 旧网络没动，容器还挂在上面跑着（假 Docker 的网络查询只列在跑的容器）。
    let body = docker.network(&network).unwrap();
    let subnet: Ipv4Net = body["IPAM"]["Config"][0]["Subnet"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(subnet.to_string(), "172.18.0.0/16");
    let containers = docker.containers();
    assert!(
        containers
            .iter()
            .any(|name| name == "wonton-app-test--samba")
            && containers.iter().any(|name| name == "wonton-app-test--web"),
        "旧容器应该一个没删：{containers:?}"
    );
    let view = m
        .list()
        .into_iter()
        .find(|view| view.id == id)
        .expect("插件已装");
    let PluginStatus::Degraded { reason } = &view.status else {
        panic!("重建失败的插件应标为异常：{:?}", view.status)
    };
    assert!(reason.contains("注入的建网失败"), "{reason}");
    let volume = Instance::default().volume(&id.slug(), "data");
    assert!(docker.volumes().iter().any(|name| name == &volume));

    // 预留放掉了：再装一个插件，落在刚才预留出来的第一个空闲段上。
    let next = "services:\n  web:\n    image: example/web\nx-wonton:\n  spec: '0.5'\n  \
        id: app.other\n  version: 1.0.0\n  kind: app\n  title: {zh: 另一个, en: Other}\n  \
        description: {zh: 测试, en: Test}\n  permissions: []";
    m.install(next, &[], super::langs()).await.unwrap();
    let other = docker
        .network(&Instance::default().network("app-other"))
        .unwrap();
    let other_subnet: Ipv4Net = other["IPAM"]["Config"][0]["Subnet"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(other_subnet, "10.213.0.0/26".parse().unwrap());
}
