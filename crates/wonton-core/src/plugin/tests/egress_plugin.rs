use std::sync::{Arc, Mutex};

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
    for state in [
        json!({"Status": "exited", "Running": false}),
        json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}}),
        json!({"Status": "running", "Running": true, "Health": {"Status": "unhealthy"}}),
    ] {
        docker.set_state(name, state);
        m.probe_egress(&Languages::default()).await.unwrap();
        assert_eq!(
            m.egress_problem().unwrap().reason,
            EgressFailure::ProxyUnreachable
        );
        let app = consumer("app.out");
        let preview = m
            .preview_install(
                &PluginPackage::parse(&app, &super::langs()).unwrap(),
                &langs,
            )
            .await
            .unwrap();
        assert!(preview.blocked.is_some());
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
    }
    m.engine.remove_container(name).await.unwrap();
    m.probe_egress(&Languages::default()).await.unwrap();
    assert_eq!(
        m.egress_problem().unwrap().reason,
        EgressFailure::ProxyUnreachable
    );
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
        false,
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
        false,
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
        format!("localhost,web,127.0.0.1,::1,.wonton.internal,auth.wonton.localhost,hub,{private}")
    );
    let hub = env_of_container(&docker, "wonton-app-out--hub");
    assert_eq!(
        hub["NO_PROXY"],
        format!("localhost,127.0.0.1,::1,.wonton.internal,auth.wonton.localhost,hub,web,{private}")
    );
}

#[tokio::test]
async fn egress_plugin_without_an_upstream_warns_but_does_not_block_installation() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let script = Arc::new(Mutex::new(Scripted {
        via_relay: Some("TLS handshake failed".to_owned()),
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
    assert_eq!(
        selection.unwrap().unwrap_err().kind(),
        "egress_proxy_unavailable"
    );
    assert_eq!(
        m.egress_settings(&Languages::default()).unwrap().source,
        EgressProxySource::Settings
    );
}
