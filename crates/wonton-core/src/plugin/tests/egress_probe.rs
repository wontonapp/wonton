//! 代理连不上时怎么报错（ADR-0023）：先经出口容器探，失败了再查原因；挡预览和安装，只有“代理本身通、连不上外面”不挡。

use std::sync::{Arc, Mutex};

use serde_json::json;
use wonton_protocol::{EgressFailure, EgressProblem, EgressProxyUpdate};

use super::super::egress::address::ProxyAddress;
use super::super::egress::probe::{Prober, Scripted};
use super::super::egress::settings::Egress;
use super::super::fake_docker::FakeDocker;
use super::super::lan::Lan;
use super::super::lan::detect::Detector;
use super::super::languages::Languages;
use super::super::manifest::Permission;
use super::super::{InstallError, PluginManager, PluginPackage};
use super::lan::{Host, add_default_bridge, office};
use super::manager::{manager_in_container, manager_with};
use super::package;

const RELAY: &str = "wonton-egress";
const CHECK: &str = "wonton-egress-check";

type Script = Arc<Mutex<Scripted>>;

fn problem(reason: EgressFailure, message: &str) -> Option<EgressProblem> {
    Some(EgressProblem {
        reason,
        message: message.to_owned(),
    })
}

fn out_app() -> String {
    package(
        "  web:\n    image: nginx",
        "  permissions: [network.egress]",
    )
}

fn closed_app() -> String {
    package("  web:\n    image: nginx", "  permissions: []")
}

fn granted(compose: &str) -> Vec<Permission> {
    PluginPackage::parse(compose).unwrap().manifest.permissions
}

fn scripted(env: Option<&str>) -> (Egress, Script) {
    let script = Script::default();
    let env = env.map(|text| ProxyAddress::parse(text).unwrap());
    let egress = Egress::new(env, None).with_prober(Prober::Fixed(script.clone()));
    (egress, script)
}

struct Rig {
    docker: FakeDocker,
    m: Arc<PluginManager>,
    script: Script,
    _dir: tempfile::TempDir,
}

impl Rig {
    async fn new(in_container: bool, env: Option<&str>) -> Self {
        let (docker, engine) = FakeDocker::start().await;
        add_default_bridge(&docker);
        let dir = tempfile::tempdir().unwrap();
        let host: Host = Arc::new(Mutex::new(Ok(office())));
        let lan = Lan::new(Detector::Fixed(host), None);
        let m = if in_container {
            docker.add_image_container("wonton-core", "wonton/core:test");
            manager_in_container(engine, &dir)
        } else {
            manager_with(engine, &dir)
        };
        let (egress, script) = scripted(env);
        Self {
            docker,
            m: Arc::new(m.with_lan(lan).with_egress(egress)),
            script,
            _dir: dir,
        }
    }

    async fn proxied() -> Self {
        Self::new(false, Some("http://proxy.lan:7899")).await
    }

    fn fail_relay(&self, direct: Option<EgressProblem>) {
        let mut script = self.script.lock().unwrap();
        script.via_relay = Some("503".to_owned());
        script.direct = direct;
    }

    async fn blocked(&self, compose: &str) -> Option<String> {
        let package = PluginPackage::parse(compose).unwrap();
        let langs = Languages::default();
        let preview = self.m.preview_install(&package, &langs).await;
        preview.unwrap().blocked
    }

    async fn install(&self, compose: &str) -> Result<(), InstallError> {
        self.m.install(compose, &granted(compose)).await.map(drop)
    }
}

#[tokio::test]
async fn a_proxy_that_cannot_be_reached_blocks_preview_and_install_with_its_reason() {
    let rig = Rig::proxied().await;
    rig.fail_relay(problem(EgressFailure::ProxyUnreachable, "5 秒没响应"));
    let app = out_app();
    assert_eq!(rig.blocked(&app).await.as_deref(), Some("5 秒没响应"));
    match rig.install(&app).await {
        Err(InstallError::EgressProxyUnavailable { reason, message }) => {
            assert_eq!(reason, EgressFailure::ProxyUnreachable);
            assert_eq!(message, "5 秒没响应");
        }
        other => panic!("{other:?}"),
    }
    assert!(rig.docker.created("wonton-app-test--web").is_none());
    let closed = closed_app();
    assert_eq!(rig.blocked(&closed).await, None);
    rig.install(&closed).await.unwrap();
}

#[tokio::test]
async fn a_proxy_that_only_cannot_reach_outside_warns_and_lets_the_install_through() {
    let rig = Rig::proxied().await;
    let upstream = problem(EgressFailure::UpstreamFailed, "它连不上外面");
    rig.fail_relay(upstream);
    let app = out_app();
    let package = PluginPackage::parse(&app).unwrap();
    let langs = Languages::default();
    let preview = rig.m.preview_install(&package, &langs).await.unwrap();
    assert_eq!(preview.blocked, None);
    assert!(preview.notices.iter().any(|n| n.text == "它连不上外面"));
    rig.install(&app).await.unwrap();
}

#[tokio::test]
async fn the_relay_failing_while_the_proxy_answers_points_at_the_relay() {
    let rig = Rig::proxied().await;
    rig.fail_relay(None);
    rig.m.probe_egress().await.unwrap();
    let found = rig.m.egress_problem().unwrap();
    assert_eq!(found.reason, EgressFailure::RelayFailed);
    assert!(found.message.contains("503"), "{}", found.message);
    assert!(found.message.contains(RELAY), "{}", found.message);
}

#[tokio::test]
async fn a_relay_image_that_cannot_be_pulled_is_reported_with_dockers_words() {
    let rig = Rig::proxied().await;
    rig.docker.fail_pulls(Some("pull access denied"));
    rig.m.probe_egress().await.unwrap();
    let found = rig.m.egress_problem().unwrap();
    assert_eq!(found.reason, EgressFailure::RelayFailed);
    assert!(
        found.message.contains("pull access denied"),
        "{}",
        found.message
    );
}

#[tokio::test]
async fn the_cores_own_path_is_reported_apart_and_does_not_block() {
    let rig = Rig::proxied().await;
    rig.script.lock().unwrap().direct = problem(EgressFailure::ProxyRejected, "回 403");
    let app = out_app();
    assert_eq!(rig.blocked(&app).await, None);
    rig.install(&app).await.unwrap();
    let settings = rig.m.egress_settings(&Languages::default()).unwrap();
    assert_eq!(settings.plugin_path.unwrap().problem, None);
    let core = settings.core_path.unwrap().problem.unwrap();
    assert_eq!(core.reason, EgressFailure::ProxyRejected);
}

#[tokio::test]
async fn a_recent_result_is_reused_and_nothing_is_probed_without_a_proxy() {
    let rig = Rig::proxied().await;
    let app = out_app();
    assert_eq!(rig.blocked(&app).await, None);
    // 一分钟内不再探：预览和紧跟着的安装不用多等一次。
    rig.fail_relay(problem(EgressFailure::ProxyUnreachable, "没响应"));
    assert_eq!(rig.blocked(&app).await, None);

    let rig = Rig::new(false, None).await;
    rig.fail_relay(problem(EgressFailure::ProxyUnreachable, "没响应"));
    assert_eq!(rig.blocked(&app).await, None);
    assert_eq!(rig.m.egress_problem(), None);
    assert!(rig.docker.created(RELAY).is_none());
}

#[tokio::test]
async fn in_a_container_the_reason_comes_from_a_locked_down_one_shot_on_the_relay_network() {
    let rig = Rig::new(true, Some("http://me:s3cret@proxy.lan:7899")).await;
    rig.script.lock().unwrap().via_relay = Some("503".to_owned());
    let found = json!({"reason": "proxy_rejected", "message": "回 407"}).to_string();
    rig.docker.set_output(CHECK, 1, &found, "");
    rig.m.probe_egress().await.unwrap();
    let found = rig.m.egress_problem().unwrap();
    assert_eq!(found.reason, EgressFailure::ProxyRejected);
    assert_eq!(found.message, "回 407");

    let created = rig.docker.created(CHECK).unwrap();
    assert_eq!(created["Image"], "wonton/core:test");
    assert_eq!(created["Cmd"], json!(["egress-proxy-check"]));
    assert_eq!(
        created["Env"],
        json!(["WONTON_EGRESS_CHECK_PROXY=http://me:s3cret@proxy.lan:7899"])
    );
    assert_eq!(created["User"], "65534:65534");
    let host = &created["HostConfig"];
    assert_eq!(host["NetworkMode"], RELAY);
    assert_eq!(host["CapDrop"], json!(["ALL"]));
    assert_eq!(host["ReadonlyRootfs"], true);
    assert_eq!(host["SecurityOpt"], json!(["no-new-privileges"]));
    assert_eq!(
        host["ExtraHosts"],
        json!(["host.docker.internal:host-gateway"])
    );
    assert!(!rig.docker.containers().contains(&CHECK.to_owned()));
}

#[tokio::test]
async fn the_core_joins_the_relay_network_to_probe_and_leaves_when_the_proxy_is_cancelled() {
    let rig = Rig::new(true, None).await;
    let langs = Languages::default();
    let update = |url: Option<&str>| EgressProxyUpdate {
        url: url.map(str::to_owned),
        direct_cidrs: Vec::new(),
        restarts: Vec::new(),
        use_plugin: false,
    };
    rig.docker.set_output(CHECK, 0, "null", "");
    let on = rig
        .m
        .set_egress_proxy(update(Some("http://proxy.lan:1")), langs.clone());
    on.await.unwrap();
    let endpoint = rig.docker.endpoint(RELAY, "wonton-core").unwrap();
    assert_eq!(endpoint["GwPriority"], -1);

    let off = rig.m.set_egress_proxy(update(None), langs);
    off.await.unwrap();
    assert_eq!(rig.docker.endpoint(RELAY, "wonton-core"), None);
    assert!(rig.docker.networks().contains(&RELAY.to_owned()));
}

#[tokio::test]
async fn plugins_to_restart_must_match_what_the_owner_confirmed() {
    let rig = Rig::new(false, None).await;
    let langs = Languages::default();
    let app = out_app();
    rig.install(&app).await.unwrap();

    let update = |restarts: &[&str]| EgressProxyUpdate {
        url: Some("http://proxy.lan:7899".to_owned()),
        direct_cidrs: Vec::new(),
        restarts: restarts.iter().map(ToString::to_string).collect(),
        use_plugin: false,
    };
    let stale = rig.m.set_egress_proxy(update(&[]), langs.clone()).await;
    assert!(matches!(stale, Err(InstallError::Changed(_))), "{stale:?}");
    assert_eq!(rig.m.store.egress_settings().unwrap(), None);

    let saved = rig.m.set_egress_proxy(update(&["app.test"]), langs).await;
    let saved = saved.unwrap();
    // 现在要确认的是取消时要重启哪些。
    assert_eq!(saved.restarts[0].id, "app.test");
    assert_eq!(saved.url.as_deref(), Some("http://proxy.lan:7899"));
    let web = rig.docker.env("wonton-app-test--web");
    assert!(
        web.iter()
            .any(|v| v == "HTTP_PROXY=http://egress.wonton.internal:3128")
    );
}

#[tokio::test]
async fn sending_back_the_masked_address_keeps_the_password() {
    let rig = Rig::new(false, None).await;
    let langs = Languages::default();
    let url = "http://me:s3cret@proxy.lan:7899";
    let update = |url: &str, direct: &[&str]| EgressProxyUpdate {
        url: Some(url.to_owned()),
        direct_cidrs: direct.iter().map(ToString::to_string).collect(),
        restarts: Vec::new(),
        use_plugin: false,
    };
    let saved = rig.m.set_egress_proxy(update(url, &[]), langs.clone());
    let masked = saved.await.unwrap().url.unwrap();
    assert!(!masked.contains("s3cret"), "{masked}");

    let edit = update(&masked, &["100.64.0.0/10"]);
    rig.m.set_egress_proxy(edit, langs).await.unwrap();
    let stored = rig.m.store.egress_settings().unwrap().unwrap();
    assert_eq!(stored.url.as_deref(), Some(url));
    assert_eq!(stored.direct_cidrs.len(), 1);
}

#[tokio::test]
async fn editing_the_direct_list_under_an_env_address_leaves_what_was_detected_alone() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let host: Host = Arc::new(Mutex::new(Ok(office())));
    let address = |text| Some(ProxyAddress::parse(text).unwrap());
    let egress = Egress::new(
        address("http://env.lan:1"),
        address("http://detected.lan:1"),
    );
    let m = manager_with(engine, &dir)
        .with_lan(Lan::new(Detector::Fixed(host), None))
        .with_egress(egress);
    let edit = EgressProxyUpdate {
        url: Some("http://env.lan:1".to_owned()),
        direct_cidrs: vec!["100.64.0.0/10".to_owned()],
        restarts: Vec::new(),
        use_plugin: false,
    };
    let m = Arc::new(m);
    m.set_egress_proxy(edit, Languages::default())
        .await
        .unwrap();
    let stored = m.store.egress_settings().unwrap().unwrap();
    // 不写成“主人清空了”：环境变量取消后，检测到的地址接着生效。
    assert_eq!(stored.url.as_deref(), Some("http://detected.lan:1"));
}

#[tokio::test]
async fn a_direct_segment_on_a_docker_network_is_refused() {
    let rig = Rig::proxied().await;
    rig.m.reconcile_relay().await.unwrap();
    // 假 Docker 给没指定网段的网络分配的。
    let subnet = "172.30.0.0/24".to_owned();
    let update = EgressProxyUpdate {
        url: Some("http://proxy.lan:7899".to_owned()),
        direct_cidrs: vec![subnet],
        restarts: Vec::new(),
        use_plugin: false,
    };
    let found = rig.m.set_egress_proxy(update, Languages::default()).await;
    assert!(matches!(found, Err(InstallError::Invalid(_))), "{found:?}");
}
