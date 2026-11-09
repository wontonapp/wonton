//! 出口容器（ADR-0023）：配了代理才有，出网插件的网络都接着它，名单跟着网络和宿主的网段走，取消时只删容器。

use std::collections::BTreeSet;
use std::fs;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::super::egress::address::ProxyAddress;
use super::super::egress::gost::IMAGE;
use super::super::egress::settings::{Egress, StoredEgress};
use super::super::fake_docker::FakeDocker;
use super::super::lan::Lan;
use super::super::lan::detect::Detector;
use super::super::lan::probe::HostNetwork;
use super::super::languages::Languages;
use super::super::manifest::Permission;
use super::super::names::{Instance, container_name};
use super::super::{PluginId, PluginManager, PluginPackage};
use super::lan::{Host, add_default_bridge, office};
use super::manager::{manager_in_instance, manager_with};
use super::{env_of_container, package};

const RELAY: &str = "wonton-egress";
const NETWORK: &str = "wonton-app-test";

fn egress_app() -> String {
    package(
        "  web:\n    image: nginx",
        "  permissions: [network.egress]",
    )
}

fn closed_app() -> String {
    package("  web:\n    image: nginx", "  permissions: []")
}

fn granted(compose: &str) -> Vec<Permission> {
    PluginPackage::parse(compose, &super::langs())
        .unwrap()
        .manifest
        .permissions
}

fn proxy(text: &str) -> Egress {
    Egress::new(
        Some(ProxyAddress::parse(text, &Default::default()).unwrap()),
        None,
    )
}

struct Rig {
    docker: FakeDocker,
    m: Arc<PluginManager>,
    host: Host,
    dir: tempfile::TempDir,
}

impl Rig {
    async fn new(egress: Egress) -> Self {
        let (docker, engine) = FakeDocker::start().await;
        add_default_bridge(&docker);
        let dir = tempfile::tempdir().unwrap();
        let host: Host = Arc::new(Mutex::new(Ok(office())));
        let lan = Lan::new(Detector::Fixed(host.clone()), None);
        let m = manager_with(engine, &dir).with_lan(lan).with_egress(egress);
        Self {
            docker,
            m: Arc::new(m),
            host,
            dir,
        }
    }

    fn file(&self, name: &str) -> String {
        let path = self.dir.path().join("plugins/egress/relay").join(name);
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}：{e}", path.display()))
    }

    fn lines(&self, name: &str) -> BTreeSet<String> {
        self.file(name).lines().map(str::to_owned).collect()
    }

    fn save(&self, url: Option<&str>, direct: &[&str]) {
        let settings = StoredEgress {
            url: url.map(str::to_owned),
            direct_cidrs: direct.iter().map(|c| c.parse().unwrap()).collect(),
            use_plugin: false,
        };
        self.m.store.save_egress_settings(&settings).unwrap();
    }

    fn relay_created(&self) -> Value {
        self.docker.created(RELAY).expect("出口容器已创建")
    }

    fn relay_creations(&self) -> usize {
        let start = format!("POST /containers/{RELAY}/start");
        let writes = self.docker.writes();
        writes.iter().filter(|write| **write == start).count()
    }

    async fn install(&self, compose: &str) {
        self.m
            .install(compose, &granted(compose), super::langs())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn with_a_proxy_an_egress_plugin_is_sent_through_the_relay() {
    let rig = Rig::new(proxy("http://me:s3cret@host.docker.internal:7899")).await;
    rig.install(&egress_app()).await;

    let env = env_of_container(&rig.docker, &container_name("app-test", "web"));
    assert_eq!(env["HTTP_PROXY"], "http://egress.wonton.internal:3128");
    assert!(env["NO_PROXY"].contains(".wonton.internal"));
    assert!(env["NO_PROXY"].contains(".localhost"));

    let created = rig.relay_created();
    assert_eq!(created["Image"], IMAGE);
    assert_eq!(created["Cmd"], json!(["-C", "/etc/gost/gost.yaml"]));
    assert_eq!(created["Labels"]["wonton.managed-by"], "core");
    assert_eq!(created["Labels"]["wonton.core.component"], "wonton-egress");
    let host = &created["HostConfig"];
    assert_eq!(host["CapDrop"], json!(["ALL"]));
    assert_eq!(host["ReadonlyRootfs"], true);
    assert_eq!(host["SecurityOpt"], json!(["no-new-privileges"]));
    assert_eq!(host["NetworkMode"], RELAY);
    assert_eq!(host["Sysctls"]["net.ipv4.ip_forward"], "0");
    assert_eq!(host["RestartPolicy"]["Name"], "unless-stopped");
    let bind = host["Binds"][0].as_str().unwrap();
    assert!(
        bind.ends_with("/plugins/egress/relay:/etc/gost:ro"),
        "{bind}"
    );
    assert_eq!(created["Env"], json!(["GOST_LOGGER_LEVEL=warn"]));
    // 密码只在配置文件里：`docker inspect` 里看不到。
    assert!(!created.to_string().contains("s3cret"));
    assert!(rig.file("gost.yaml").contains("s3cret"));

    let labels = rig.docker.network(RELAY).expect("它自己的网络")["Labels"].clone();
    assert_eq!(labels["wonton.managed-by"], "core");
    assert_eq!(labels["wonton.core.component"], "wonton-egress");

    let endpoint = rig.docker.endpoint(NETWORK, RELAY).expect("接入插件网络");
    assert_eq!(endpoint["Aliases"], json!(["egress.wonton.internal"]));
    assert_eq!(endpoint["GwPriority"], -1);
}

#[tokio::test]
async fn plugins_that_do_not_go_out_never_see_the_relay() {
    let rig = Rig::new(proxy("http://host.docker.internal:7899")).await;
    rig.install(&closed_app()).await;
    let env = env_of_container(&rig.docker, &container_name("app-test", "web"));
    assert!(!env.contains_key("HTTP_PROXY"), "{env:?}");
    assert_eq!(rig.docker.endpoint(NETWORK, RELAY), None);
    assert!(rig.docker.created(RELAY).is_none());
}

#[tokio::test]
async fn without_a_proxy_nothing_changes() {
    let rig = Rig::new(Egress::default()).await;
    rig.install(&egress_app()).await;
    let env = env_of_container(&rig.docker, &container_name("app-test", "web"));
    assert!(!env.contains_key("HTTP_PROXY"), "{env:?}");
    assert!(!env.contains_key("NODE_USE_ENV_PROXY"), "{env:?}");
    assert!(rig.docker.created(RELAY).is_none());
    assert!(!rig.docker.networks().contains(&RELAY.to_owned()));
}

#[tokio::test]
async fn the_lists_let_the_host_through_and_stop_everything_else_inside() {
    let rig = Rig::new(proxy("http://host.docker.internal:7899")).await;
    rig.save(None, &["100.64.0.0/10"]);
    rig.install(&egress_app()).await;
    let direct = rig.lines("direct.txt");
    // 选出的地址 192.168.0.10 所在的网段，主人多放行的网段，假 Docker 里网桥的网关。
    let mut expected = BTreeSet::from([
        "100.64.0.0/10".to_owned(),
        "172.17.0.1/32".to_owned(),
        "192.168.0.0/24".to_owned(),
    ]);
    for name in [NETWORK, RELAY] {
        expected.insert(format!(
            "{}/32",
            rig.m.engine.network_gateway(name).await.unwrap()
        ));
    }
    assert_eq!(direct, expected);
    let deny = rig.lines("deny.txt");
    assert!(deny.contains("127.0.0.0/8"), "{deny:?}");
    assert!(deny.contains("*.wonton.internal"), "{deny:?}");
    assert!(deny.contains("wonton.localhost"), "{deny:?}");
    assert!(deny.contains("*.wonton.localhost"), "{deny:?}");
    assert!(!deny.contains("192.168.0.0/16"), "{deny:?}");
    assert!(!deny.contains("100.64.0.0/10"), "{deny:?}");
    assert!(deny.contains("192.168.1.0/24"), "{deny:?}");
}

#[tokio::test]
async fn a_segment_overlapping_a_network_the_relay_is_on_stays_denied() {
    let rig = Rig::new(proxy("http://host.docker.internal:7899")).await;
    rig.m.reconcile_relay().await.unwrap();
    let body = rig.docker.network(RELAY).unwrap();
    let subnet = body["IPAM"]["Config"][0]["Subnet"].as_str().unwrap();
    let net: ipnet::Ipv4Net = subnet.parse().unwrap();
    rig.save(None, &[subnet]);
    rig.install(&egress_app()).await;
    let direct = rig.lines("direct.txt");
    assert!(!direct.contains(subnet), "{direct:?}");
    for name in [NETWORK, RELAY] {
        assert!(
            direct.contains(&format!(
                "{}/32",
                rig.m.engine.network_gateway(name).await.unwrap()
            )),
            "{direct:?}"
        );
    }
    let denied = std::net::Ipv4Addr::from(net.network().to_bits() + 2);
    assert!(rig.lines("deny.txt").contains(&format!("{denied}/31")));
}

#[tokio::test]
async fn when_the_lan_address_moves_only_the_lists_change() {
    let rig = Rig::new(proxy("http://host.docker.internal:7899")).await;
    rig.install(&egress_app()).await;
    assert!(rig.lines("direct.txt").contains("192.168.0.0/24"));

    let moved: HostNetwork = {
        let mut network = office();
        network.interfaces[0].addresses[0].ip = "192.168.5.7".parse().unwrap();
        network
    };
    *rig.host.lock().unwrap() = Ok(moved);
    rig.m.lan_pass(&Languages::default()).await.unwrap();
    let direct = rig.lines("direct.txt");
    assert!(direct.contains("192.168.5.0/24"), "{direct:?}");
    assert!(!direct.contains("192.168.0.0/24"), "{direct:?}");
    assert_eq!(rig.relay_creations(), 1);
}

#[tokio::test]
async fn uninstalling_detaches_the_relay_first_and_keeps_it_for_the_others() {
    let rig = Rig::new(proxy("http://host.docker.internal:7899")).await;
    rig.install(&egress_app()).await;
    let id = PluginId::parse("app.test").unwrap();
    rig.m.uninstall(&id, false, &super::langs()).await.unwrap();
    let writes = rig.docker.writes();
    let position = |needle: &str| {
        writes
            .iter()
            .rposition(|write| write == needle)
            .unwrap_or_else(|| panic!("{needle}：{writes:?}"))
    };
    let disconnect = position("POST /networks/wonton-app-test/disconnect");
    assert!(disconnect < position("DELETE /networks/wonton-app-test"));
    assert!(!rig.docker.networks().contains(&NETWORK.to_owned()));
    assert!(rig.docker.containers().contains(&RELAY.to_owned()));
}

#[tokio::test]
async fn a_failed_install_takes_the_relay_off_the_network_it_removes() {
    let rig = Rig::new(proxy("http://host.docker.internal:7899")).await;
    // 记录的位置被一个目录占着：部署的最后一步必然失败，回滚。
    fs::create_dir_all(rig.dir.path().join("plugins/app.test.json")).unwrap();
    let compose = egress_app();
    let err = rig
        .m
        .install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "storage", "{err}");
    assert!(!rig.docker.networks().contains(&NETWORK.to_owned()));
    assert_eq!(rig.docker.endpoint(NETWORK, RELAY), None);
}

#[tokio::test]
async fn an_image_that_cannot_be_pulled_stops_the_install_with_dockers_words() {
    let rig = Rig::new(proxy("http://host.docker.internal:7899")).await;
    rig.docker.fail_pulls(Some("toomanyrequests"));
    let compose = egress_app();
    let err = rig
        .m
        .install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("toomanyrequests"), "{err}");
    assert!(rig.docker.created(RELAY).is_none());
}

#[tokio::test]
async fn a_container_or_network_that_is_not_ours_is_left_alone() {
    let rig = Rig::new(proxy("http://host.docker.internal:7899")).await;
    rig.docker.add_stranger_container(RELAY, json!({}));
    let compose = egress_app();
    let err = rig
        .m
        .install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
    assert!(rig.docker.containers().contains(&RELAY.to_owned()));

    let rig = Rig::new(proxy("http://host.docker.internal:7899")).await;
    rig.docker.add_unlabeled_network(RELAY);
    let err = rig
        .m
        .install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
}

#[tokio::test]
async fn configuring_a_proxy_later_attaches_the_plugins_already_installed() {
    let rig = Rig::new(Egress::default()).await;
    rig.install(&egress_app()).await;
    assert_eq!(rig.docker.endpoint(NETWORK, RELAY), None);
    rig.save(Some("http://proxy:1"), &[]);
    rig.m.reconcile_relay().await.unwrap();
    let endpoint = rig.docker.endpoint(NETWORK, RELAY).expect("接入插件网络");
    assert_eq!(endpoint["GwPriority"], -1);
}

#[tokio::test]
async fn a_new_address_recreates_only_the_relay_and_the_same_one_does_nothing() {
    let rig = Rig::new(Egress::default()).await;
    rig.save(Some("http://proxy:1"), &[]);
    rig.install(&egress_app()).await;
    rig.m.reconcile_relay().await.unwrap();
    assert_eq!(rig.relay_creations(), 1, "地址没变不重建");

    let web = container_name("app-test", "web");
    let plugin_creations = |rig: &Rig| {
        let start = format!("POST /containers/{web}/start");
        let writes = rig.docker.writes();
        writes.iter().filter(|write| **write == start).count()
    };
    let before = plugin_creations(&rig);
    rig.save(Some("http://proxy:2"), &[]);
    rig.m.reconcile_relay().await.unwrap();
    assert_eq!(rig.relay_creations(), 2);
    assert!(rig.file("gost.yaml").contains("proxy:2"));
    assert_eq!(plugin_creations(&rig), before, "插件不重启");
    assert!(rig.docker.endpoint(NETWORK, RELAY).is_some(), "重新接入");
}

#[tokio::test]
async fn a_relay_that_died_is_brought_back() {
    let rig = Rig::new(Egress::default()).await;
    rig.save(Some("http://proxy:1"), &[]);
    rig.install(&egress_app()).await;
    rig.docker
        .set_state(RELAY, json!({"Status": "exited", "Running": false}));
    rig.m.reconcile_relay().await.unwrap();
    assert_eq!(rig.relay_creations(), 2);
}

#[tokio::test]
async fn a_relay_on_an_older_image_is_replaced() {
    let rig = Rig::new(Egress::default()).await;
    rig.save(Some("http://proxy:1"), &[]);
    rig.install(&egress_app()).await;
    let body = json!({"Image": "gogost/gost:old", "Labels": rig.relay_created()["Labels"]});
    rig.docker.add_stranger_container(RELAY, body);
    rig.m.reconcile_relay().await.unwrap();
    assert_eq!(rig.relay_created()["Image"], IMAGE);
}

#[tokio::test]
async fn registry_mirror_relay_with_the_same_digest_is_not_recreated() {
    let rig = Rig::new(Egress::default()).await;
    rig.save(Some("http://proxy:1"), &[]);
    rig.install(&egress_app()).await;
    let digest = IMAGE.split_once('@').unwrap().1;
    let image = format!("mirror.example/gogost/gost@{digest}");
    let body = json!({"Image": image, "Labels": rig.relay_created()["Labels"]});
    rig.docker.add_stranger_container(RELAY, body);
    let before = rig.relay_creations();
    rig.m.reconcile_relay().await.unwrap();
    rig.m.reconcile_relay().await.unwrap();
    assert_eq!(rig.relay_creations(), before);
    assert_eq!(rig.relay_created()["Image"], image);
}

#[tokio::test]
async fn cancelling_removes_the_relay_and_its_password_but_not_its_network() {
    let rig = Rig::new(Egress::default()).await;
    rig.save(Some("http://me:s3cret@proxy:1"), &[]);
    rig.install(&egress_app()).await;
    rig.save(None, &[]);
    rig.m.reconcile_relay().await.unwrap();
    assert!(!rig.docker.containers().contains(&RELAY.to_owned()));
    assert!(rig.docker.networks().contains(&RELAY.to_owned()));
    assert!(!rig.dir.path().join("plugins/egress/relay").exists());
    // 再配上时沿用同一个网络。
    rig.save(Some("http://proxy:2"), &[]);
    rig.m.reconcile_relay().await.unwrap();
    assert!(rig.docker.containers().contains(&RELAY.to_owned()));
}

#[tokio::test]
async fn each_core_on_a_shared_docker_has_its_own_relay() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_in_instance(engine, &dir, "alice2").with_egress(proxy("http://proxy:1"));
    let compose = egress_app();
    m.install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap();
    let relay = "wonton-alice2--egress";
    let created = docker.created(relay).expect("这套核心的出口容器");
    assert_eq!(created["Labels"]["wonton.instance"], "alice2");
    assert!(docker.networks().contains(&relay.to_owned()));
    assert!(docker.endpoint("wonton-alice2--app-test", relay).is_some());
}

#[tokio::test]
async fn a_leftover_network_with_the_relay_on_it_can_still_be_cleared() {
    let rig = Rig::new(proxy("http://proxy:1")).await;
    rig.docker
        .add_network(NETWORK, "app.test", json!({"Internal": false}));
    rig.docker.add_endpoint(
        NETWORK,
        RELAY,
        json!({"Aliases": ["egress.wonton.internal"]}),
    );
    let labels = Instance::default().component_labels();
    let body = json!({"Image": IMAGE, "Labels": labels});
    rig.docker.add_stranger_container(RELAY, body);
    rig.install(&egress_app()).await;
    assert!(rig.docker.endpoint(NETWORK, RELAY).is_some());
}
