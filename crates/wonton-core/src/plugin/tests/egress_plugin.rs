use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use ipnet::Ipv4Net;

use serde_json::json;
use wonton_protocol::{EgressFailure, EgressProxySource, EgressProxyUpdate};

use super::super::egress::probe::{Prober, Scripted};
use super::super::egress::settings::Egress;
use super::super::fake_docker::FakeDocker;
use super::super::lan::{Lan, detect::Detector};
use super::super::languages::Languages;
use super::super::{PluginId, PluginManager, PluginPackage, PluginStatus};
use super::lan::{Host, add_default_bridge, office};
use super::manager::{manager_in_container, manager_in_instance, manager_with};
use super::{env_of_container, package};

pub(super) fn proxy_app(id: &str) -> String {
    package(
        "  proxy:\n    image: nginx",
        "  provides:\n    egress_proxy: {service: proxy, port: 7890}\n  permissions: [network.egress, egress.proxy]",
    ).replace("id: app.test", &format!("id: {id}"))
}

fn consumer(id: &str) -> String {
    package(
        "  web:\n    image: nginx",
        "  permissions: [network.egress]",
    )
    .replace("id: app.test", &format!("id: {id}"))
}

async fn install(m: &PluginManager, compose: &str) {
    let grants = PluginPackage::parse(compose, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    m.install(compose, &grants, super::langs()).await.unwrap();
}

fn update(url: Option<&str>, use_plugin: bool, restarts: &[&str]) -> EgressProxyUpdate {
    EgressProxyUpdate {
        url: url.map(str::to_owned),
        use_plugin,
        direct_cidrs: Vec::new(),
        restarts: restarts.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn lan() -> Lan {
    let host: Host = Arc::new(Mutex::new(Ok(office())));
    Lan::new(Detector::Fixed(host), None)
}

fn starts(docker: &FakeDocker, name: &str) -> usize {
    docker
        .writes()
        .iter()
        .filter(|s| *s == &format!("POST /containers/{name}/start"))
        .count()
}

#[tokio::test]
async fn egress_plugin_selection_switches_only_relay_between_providers_and_survives_restore() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_in_instance(engine.clone(), &dir, "emc").with_lan(lan()));
    install(&m, &proxy_app("app.proxy")).await;
    install(&m, &consumer("app.out")).await;
    let langs = Languages::from_header(Some("zh"));
    let unselected = m.egress_settings(&langs).unwrap();
    assert_eq!(unselected.source, EgressProxySource::Unset);
    assert_eq!(unselected.provider.unwrap().id, "app.proxy");
    assert_eq!(
        unselected
            .restarts
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["app.out"]
    );
    let changed = m
        .set_egress_proxy(update(None, true, &[]), langs.clone())
        .await
        .unwrap_err();
    assert_eq!(changed.kind(), "changed");
    let selected = m
        .set_egress_proxy(update(None, true, &["app.out"]), langs.clone())
        .await
        .unwrap();
    assert_eq!(selected.source, EgressProxySource::Plugin);
    assert_eq!(selected.url, None);
    let provider = selected.provider.unwrap();
    assert_eq!(provider.id, "app.proxy");
    assert_eq!(provider.title, "测试");
    let proxy = "wonton-emc--app-proxy--proxy";
    let out = "wonton-emc--app-out--web";
    let relay = "wonton-emc--egress";
    assert!(!env_of_container(&docker, proxy).contains_key("HTTP_PROXY"));
    assert_eq!(
        env_of_container(&docker, out)["HTTP_PROXY"],
        super::super::egress::PROXY_URL
    );
    assert_eq!(starts(&docker, proxy), 1);
    assert_eq!(starts(&docker, out), 2);
    assert!(docker.endpoint("wonton-emc--app-proxy", relay).is_some());
    assert_eq!(
        m.egress_config().unwrap().url.unwrap().text(),
        format!("http://{proxy}:7890")
    );
    let stored = m.store.egress_settings().unwrap().unwrap();
    assert!(stored.use_plugin);
    assert_eq!(stored.url, None);
    let restarted = Arc::new(
        manager_in_instance(engine, &dir, "emc")
            .with_lan(lan())
            .with_egress(Egress::default()),
    );
    restarted.restore().await.unwrap();
    let restored = restarted.egress_settings(&langs).unwrap();
    assert_eq!(restored.source, EgressProxySource::Plugin);
    assert_eq!(restored.provider.unwrap().id, "app.proxy");
    assert!(
        restarted
            .list()
            .iter()
            .all(|p| p.status == PluginStatus::Running)
    );
    let manual = restarted
        .set_egress_proxy(
            update(Some("http://proxy.lan:1"), false, &[]),
            langs.clone(),
        )
        .await
        .unwrap();
    assert_eq!(manual.source, EgressProxySource::Settings);
    assert_eq!(starts(&docker, out), 2);
    assert_eq!(starts(&docker, proxy), 1);
    assert_eq!(starts(&docker, relay), 2);
    restarted
        .set_egress_proxy(update(None, true, &[]), langs.clone())
        .await
        .unwrap();
    assert_eq!(starts(&docker, out), 2);
    assert_eq!(starts(&docker, relay), 3);
    restarted
        .set_egress_proxy(update(None, false, &["app.out"]), langs)
        .await
        .unwrap();
    assert_eq!(starts(&docker, out), 3);
    assert_eq!(starts(&docker, proxy), 1);
    assert!(!env_of_container(&docker, out).contains_key("HTTP_PROXY"));
    restarted
        .uninstall(
            &PluginId::parse("app.proxy").unwrap(),
            false,
            &super::langs(),
        )
        .await
        .unwrap();
}

/// 宿主网络的服务过不了别名：代理写出口容器在插件网络里的固定地址（核心地址的前一个，[`egress_address`]）。
#[tokio::test]
async fn host_networked_services_proxy_through_the_pinned_relay_address() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_in_instance(engine, &dir, "hst").with_egress(Egress::default()));
    let compose = package(
        "  esp:\n    image: esphome/esphome\n    network_mode: host",
        "  permissions: [network.egress, network.host]",
    );
    let grants = PluginPackage::parse(&compose, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    m.install(&compose, &grants, super::langs()).await.unwrap();
    m.set_egress_proxy(
        update(Some("http://proxy.lan:1"), false, &["app.test"]),
        Languages::from_header(Some("zh")),
    )
    .await
    .unwrap();

    let network = "wonton-hst--app-test";
    let request = docker.network(network).unwrap();
    let subnet: Ipv4Net = request["IPAM"]["Config"][0]["Subnet"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let address = Ipv4Addr::from(subnet.broadcast().to_bits() - 2).to_string();
    assert_eq!(
        env_of_container(&docker, "wonton-hst--app-test--esp")["HTTP_PROXY"],
        format!("http://{address}:3128")
    );
    let pinned = docker
        .endpoint(network, &m.cfg.instance.egress())
        .expect("出口容器接在插件网络上");
    assert_eq!(pinned["IPAMConfig"]["IPv4Address"], address.as_str());
}

#[tokio::test]
async fn egress_plugin_singleton_reserves_even_during_concurrent_installs() {
    let (_, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let first = proxy_app("app.first");
    let second = proxy_app("app.second");
    let grants = PluginPackage::parse(&first, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    let (a, b) = tokio::join!(
        m.install(&first, &grants, super::langs()),
        m.install(&second, &grants, super::langs())
    );
    let error = match (a, b) {
        (Ok(_), Err(e)) | (Err(e), Ok(_)) => e,
        result => panic!("只应装上一个代理插件：{result:?}"),
    };
    assert_eq!(error.kind(), "conflict");
    assert_eq!(m.list().len(), 1);
}

/// 代理连不上时：消费者的预览被拦，安装报 `egress_proxy_*`（坏是 `unavailable`、在等是 `waiting`）。
async fn assert_consumer_blocked(m: &PluginManager, langs: &Languages) {
    let app = consumer("app.out");
    let preview = m
        .preview_install(&PluginPackage::parse(&app, &super::langs()).unwrap(), langs)
        .await
        .unwrap();
    assert!(preview.blocked.is_some());
    let grants = PluginPackage::parse(&app, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    assert!(
        m.install(&app, &grants, super::langs())
            .await
            .unwrap_err()
            .kind()
            .starts_with("egress_proxy")
    );
}

#[tokio::test]
async fn a_provider_that_is_still_being_installed_blocks_with_a_waiting_reason() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan()));
    install(&m, &proxy_app("app.proxy")).await;
    let langs = Languages::default();
    m.set_egress_proxy(update(None, true, &[]), langs.clone())
        .await
        .unwrap();
    // 刚选上的出网插件自己也在装：容器还没建，插件状态是安装中。
    let id = PluginId::parse("app.proxy").unwrap();
    m.plugins.write().unwrap().get_mut(&id).unwrap().status = PluginStatus::Installing;
    m.engine
        .remove_container("wonton-app-proxy--proxy")
        .await
        .unwrap();
    m.probe_egress(&langs).await.unwrap();
    let found = m.egress_problem().unwrap();
    assert_eq!(found.reason, EgressFailure::Waiting);
    assert!(found.message.contains("Test"), "{}", found.message);
    assert!(
        found.message.contains("being installed"),
        "{}",
        found.message
    );
    // 壳按类别换标题：等待要有自己的类别，别落到“代理用不了”。
    let app = consumer("app.out");
    let grants = PluginPackage::parse(&app, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    assert_eq!(
        m.install(&app, &grants, super::langs())
            .await
            .unwrap_err()
            .kind(),
        "egress_proxy_waiting"
    );
    assert_consumer_blocked(&m, &langs).await;
}

/// 出口插件在换版本时不是坏：等它换完，别按代理坏了报。
#[tokio::test]
async fn a_provider_that_is_upgrading_still_blocks_but_as_waiting() {
    before_status(PluginStatus::Upgrading, "being upgraded").await;
}

/// 出口插件在卸载时不是坏：等它走完，别按代理坏了报。
#[tokio::test]
async fn a_provider_that_is_uninstalling_still_blocks_but_as_waiting() {
    before_status(PluginStatus::Uninstalling, "being uninstalled").await;
}

/// 把出口插件摆到 `status`、核对一次，断言挡安装且说清在等它。
async fn before_status(status: PluginStatus, expected: &str) {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan()));
    install(&m, &proxy_app("app.proxy")).await;
    let langs = Languages::default();
    m.set_egress_proxy(update(None, true, &[]), langs.clone())
        .await
        .unwrap();
    let id = PluginId::parse("app.proxy").unwrap();
    m.plugins.write().unwrap().get_mut(&id).unwrap().status = status;
    m.probe_egress(&langs).await.unwrap();
    let found = m.egress_problem().unwrap();
    assert_eq!(found.reason, EgressFailure::Waiting);
    assert!(found.message.contains("Test"), "{}", found.message);
    assert!(found.message.contains(expected), "{}", found.message);
    assert_consumer_blocked(&m, &langs).await;
}

#[tokio::test]
async fn a_provider_that_stopped_running_names_the_page_to_look_at() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan()));
    install(&m, &proxy_app("app.proxy")).await;
    let langs = Languages::default();
    m.set_egress_proxy(update(None, true, &[]), langs.clone())
        .await
        .unwrap();
    let id = PluginId::parse("app.proxy").unwrap();
    m.plugins.write().unwrap().get_mut(&id).unwrap().status = PluginStatus::Degraded {
        reason: "启动失败".to_owned(),
    };
    docker.set_state(
        "wonton-app-proxy--proxy",
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    m.probe_egress(&langs).await.unwrap();
    let found = m.egress_problem().unwrap();
    assert_eq!(found.reason, EgressFailure::ProxyUnreachable);
    assert!(found.message.contains("cannot serve"), "{}", found.message);
    assert!(found.message.contains("exited"), "{}", found.message);
    assert!(found.message.contains("exit code 1"), "{}", found.message);
    assert!(found.message.contains("logs"), "{}", found.message);
    // 真坏了是失败，不是等待。
    let app = consumer("app.out");
    let grants = PluginPackage::parse(&app, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    assert_eq!(
        m.install(&app, &grants, super::langs())
            .await
            .unwrap_err()
            .kind(),
        "egress_proxy_unavailable"
    );
    assert_consumer_blocked(&m, &langs).await;
}

#[tokio::test]
async fn egress_plugin_failures_block_consumers_but_allow_provider_install_upgrade_and_reinstall() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let script = Arc::new(Mutex::new(Scripted::default()));
    let m = Arc::new(
        manager_with(engine, &dir)
            .with_lan(lan())
            .with_egress(Egress::default().with_prober(Prober::Fixed(script.clone()))),
    );
    install(&m, &proxy_app("app.proxy")).await;
    let langs = Languages::default();
    m.set_egress_proxy(update(None, true, &[]), langs.clone())
        .await
        .unwrap();
    let name = "wonton-app-proxy--proxy";
    for (state, expected) in [
        (
            json!({"Status": "exited", "Running": false}),
            EgressFailure::ProxyUnreachable,
        ),
        (
            json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}}),
            EgressFailure::Waiting,
        ),
        (
            json!({"Status": "running", "Running": true, "Health": {"Status": "unhealthy"}}),
            EgressFailure::ProxyUnreachable,
        ),
    ] {
        docker.set_state(name, state);
        m.probe_egress(&Languages::default()).await.unwrap();
        let found = m.egress_problem().unwrap();
        assert_eq!(found.reason, expected, "{}", found.message);
        assert_consumer_blocked(&m, &langs).await;
    }
    // 状态还说在运行、容器却没了：没有谁会自己去起它，如实报出来。
    m.engine.remove_container(name).await.unwrap();
    m.probe_egress(&Languages::default()).await.unwrap();
    let found = m.egress_problem().unwrap();
    assert_eq!(found.reason, EgressFailure::ProxyUnreachable);
    assert!(found.message.contains("no container"), "{}", found.message);
    docker.set_state(
        name,
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    let upgraded = PluginPackage::parse(
        &proxy_app("app.proxy").replace("1.0.0", "1.0.1"),
        &super::langs(),
    )
    .unwrap();
    let grants = upgraded.manifest.permissions.clone();
    m.upgrade_upload(
        &PluginId::parse("app.proxy").unwrap(),
        upgraded,
        grants,
        Vec::new(),
        false,
        &[],
        &Default::default(),
        &super::langs(),
    )
    .await
    .unwrap();
    assert_eq!(m.list()[0].version, "1.0.1");
    assert!(!env_of_container(&docker, name).contains_key("HTTP_PROXY"));
    m.set_egress_proxy(update(None, false, &[]), langs)
        .await
        .unwrap();
    {
        let mut script = script.lock().unwrap();
        script.via_relay = Some("503".to_owned());
        script.direct = Some(wonton_protocol::EgressProblem {
            reason: EgressFailure::ProxyUnreachable,
            message: "broken host proxy".to_owned(),
        });
    }
    m.store
        .save_egress_settings(&super::super::egress::settings::StoredEgress {
            url: Some("http://proxy.lan:1".to_owned()),
            ..Default::default()
        })
        .unwrap();
    m.uninstall(
        &PluginId::parse("app.proxy").unwrap(),
        false,
        &super::langs(),
    )
    .await
    .unwrap();
    install(&m, &proxy_app("app.proxy")).await;
    assert!(!env_of_container(&docker, name).contains_key("HTTP_PROXY"));
}

#[tokio::test]
async fn egress_plugin_diagnosis_uses_the_provider_network() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    docker.add_image_container("wonton-core", "wonton/core:test");
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_in_container(engine, &dir).with_lan(lan()));
    install(&m, &proxy_app("app.proxy")).await;
    docker.set_output("wonton-egress-check", 0, "null", "");
    m.set_egress_proxy(update(None, true, &[]), Languages::default())
        .await
        .unwrap();
    let created = docker.created("wonton-egress-check").unwrap();
    assert_eq!(created["HostConfig"]["NetworkMode"], "wonton-app-proxy");
}

#[tokio::test]
async fn egress_plugin_upgrade_retargets_relay_without_restarting_consumers() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan()));
    install(&m, &proxy_app("app.proxy")).await;
    m.set_egress_proxy(update(None, true, &[]), Languages::default())
        .await
        .unwrap();
    install(&m, &consumer("app.out")).await;
    let upgraded = proxy_app("app.proxy")
        .replace("1.0.0", "1.0.1")
        .replace("port: 7890", "port: 7891");
    let upgraded = PluginPackage::parse(&upgraded, &super::langs()).unwrap();
    let grants = upgraded.manifest.permissions.clone();
    m.upgrade_upload(
        &PluginId::parse("app.proxy").unwrap(),
        upgraded,
        grants,
        Vec::new(),
        false,
        &[],
        &Default::default(),
        &super::langs(),
    )
    .await
    .unwrap();
    assert_eq!(m.egress_config().unwrap().url.unwrap().port, 7891);
    assert_eq!(starts(&docker, "wonton-app-out--web"), 1);
    assert_eq!(starts(&docker, "wonton-egress"), 2);
}

#[tokio::test]
async fn plugin_written_no_proxy_is_merged_and_networked_services_all_get_one() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan()));
    install(&m, &proxy_app("app.proxy")).await;
    m.set_egress_proxy(update(None, true, &[]), Languages::default())
        .await
        .unwrap();
    let out = package(
        "  web:\n    image: nginx\n    environment:\n      NO_PROXY: \"localhost,web\"\n  \
         hub:\n    image: nginx",
        "  permissions: [network.egress]",
    )
    .replace("id: app.test", "id: app.out");
    install(&m, &out).await;
    // 服务名按字典序：hub、web；私有网段在名单末尾。
    let private = ".local,.lan,.home.arpa,10.0.0.0/8,100.64.0.0/10,169.254.0.0/16,\
                   172.16.0.0/12,192.168.0.0/16";
    let web = env_of_container(&docker, "wonton-app-out--web");
    assert_eq!(web["HTTP_PROXY"], super::super::egress::PROXY_URL);
    assert_eq!(
        web["NO_PROXY"],
        format!("localhost,web,.localhost,127.0.0.1,::1,.wonton.internal,hub,{private}")
    );
    let hub = env_of_container(&docker, "wonton-app-out--hub");
    assert_eq!(
        hub["NO_PROXY"],
        format!("localhost,.localhost,127.0.0.1,::1,.wonton.internal,hub,web,{private}")
    );
}

#[tokio::test]
async fn egress_plugin_without_an_upstream_warns_but_does_not_block_installation() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let script = Arc::new(Mutex::new(Scripted {
        via_relay: Some("TLS handshake failed".to_owned()),
        via_relay_later: Vec::new(),
        direct: Some(wonton_protocol::EgressProblem {
            reason: EgressFailure::UpstreamFailed,
            message: "CONNECT succeeded; upstream TLS unavailable".to_owned(),
        }),
    }));
    let m = Arc::new(
        manager_with(engine, &dir)
            .with_lan(lan())
            .with_egress(Egress::default().with_prober(Prober::Fixed(script))),
    );
    install(&m, &proxy_app("app.proxy")).await;
    let settings = m
        .set_egress_proxy(update(None, true, &[]), Languages::default())
        .await
        .unwrap();
    assert_eq!(
        settings.plugin_path.unwrap().problem.unwrap().reason,
        EgressFailure::UpstreamFailed
    );
    install(&m, &consumer("app.out")).await;
    assert_eq!(
        env_of_container(&docker, "wonton-app-out--web")["HTTP_PROXY"],
        super::super::egress::PROXY_URL
    );
}

#[tokio::test]
async fn egress_plugin_being_uninstalled_cannot_be_selected_even_at_the_same_manual_address() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir).with_lan(lan()));
    install(&m, &proxy_app("app.proxy")).await;
    m.set_egress_proxy(
        update(Some("http://wonton-app-proxy--proxy:7890"), false, &[]),
        Languages::default(),
    )
    .await
    .unwrap();
    let hold = docker.hold("DELETE /containers/wonton-app-proxy--proxy");
    let uninstall = tokio::spawn({
        let m = m.clone();
        async move {
            m.uninstall(
                &PluginId::parse("app.proxy").unwrap(),
                false,
                &super::langs(),
            )
            .await
        }
    });
    hold.reached().await;
    let selection = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        m.set_egress_proxy(update(None, true, &[]), Languages::default()),
    )
    .await;
    hold.release();
    uninstall.await.unwrap().unwrap();
    // 出口插件在卸载，挡是等着它走完。
    assert_eq!(
        selection.unwrap().unwrap_err().kind(),
        "egress_proxy_waiting"
    );
    assert_eq!(
        m.egress_settings(&Languages::default()).unwrap().source,
        EgressProxySource::Settings
    );
}
