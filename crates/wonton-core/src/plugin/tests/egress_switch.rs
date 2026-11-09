//! 第一次配上或取消代理（ADR-0023）：出网插件的容器重建，数据不动；Bot 沿用同一台设备，令牌从旧容器读回，读不到就不动容器。

use std::sync::Arc;

use serde_json::json;

use crate::l10n::CATALOG;

use super::super::egress::settings::StoredEgress;
use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::manifest::Permission;
use super::super::names::container_name;
use super::super::{PluginId, PluginManager, PluginPackage, PluginStatus};
use super::manager::{manager_in_container, manager_with};
use super::{env_of_container, package};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{Fake, start};

fn granted(compose: &str) -> Vec<Permission> {
    PluginPackage::parse(compose, &super::langs())
        .unwrap()
        .manifest
        .permissions
}

fn app(id: &str, permissions: &str) -> String {
    package(
        "  web:\n    image: nginx\n    environment: {DATA: kept}",
        &format!("  permissions: [{permissions}]"),
    )
    .replace("id: app.test", &format!("id: {id}"))
}

fn id(text: &str) -> PluginId {
    PluginId::parse(text).unwrap()
}

fn set_proxy(m: &PluginManager, url: Option<&str>) {
    let settings = StoredEgress {
        url: url.map(str::to_owned),
        doh_url: None,
        direct_cidrs: Vec::new(),
        use_plugin: false,
    };
    m.store.save_egress_settings(&settings).unwrap();
}

fn starts(docker: &FakeDocker, container: &str) -> usize {
    let start = format!("POST /containers/{container}/start");
    let writes = docker.writes();
    writes.iter().filter(|write| **write == start).count()
}

fn status(m: &PluginManager, plugin: &str) -> PluginStatus {
    m.status_of(&id(plugin)).unwrap()
}

#[tokio::test]
async fn turning_the_proxy_on_and_off_rebuilds_only_the_plugins_that_go_out() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let out = app("app.out", "network.egress");
    let closed = app("app.closed", "");
    m.install(&out, &granted(&out), super::langs())
        .await
        .unwrap();
    m.install(&closed, &granted(&closed), super::langs())
        .await
        .unwrap();
    let (out_web, closed_web) = (
        container_name("app-out", "web"),
        container_name("app-closed", "web"),
    );

    let restarts = m.egress_restarts(true, &Languages::default()).unwrap();
    let ids: Vec<&str> = restarts.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["app.out"]);
    assert_eq!(restarts[0].title, "Test");

    set_proxy(&m, Some("http://proxy:1"));
    m.apply_egress().await.unwrap();
    let env = env_of_container(&docker, &out_web);
    assert_eq!(env["HTTP_PROXY"], "http://egress.wonton.internal:3128");
    assert_eq!(env["DATA"], "kept");
    assert_eq!(starts(&docker, &out_web), 2);
    assert_eq!(starts(&docker, &closed_web), 1);
    assert_eq!(status(&m, "app.out"), PluginStatus::Running);
    assert!(m.store.load(&id("app.out")).unwrap().unwrap().proxied);
    assert!(
        m.egress_restarts(true, &Languages::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        m.egress_restarts(false, &Languages::default())
            .unwrap()
            .len(),
        1
    );

    // 只改地址：插件不重启。
    set_proxy(&m, Some("http://proxy:2"));
    m.apply_egress().await.unwrap();
    assert_eq!(starts(&docker, &out_web), 2);

    set_proxy(&m, None);
    m.apply_egress().await.unwrap();
    let env = env_of_container(&docker, &out_web);
    assert!(!env.contains_key("HTTP_PROXY"), "{env:?}");
    assert_eq!(starts(&docker, &out_web), 3);
    assert!(!m.store.load(&id("app.out")).unwrap().unwrap().proxied);
}

#[tokio::test]
async fn a_plugin_installed_with_the_proxy_on_is_not_rebuilt() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    set_proxy(&m, Some("http://proxy:1"));
    let out = app("app.out", "network.egress");
    m.install(&out, &granted(&out), super::langs())
        .await
        .unwrap();
    m.apply_egress().await.unwrap();
    assert_eq!(starts(&docker, &container_name("app-out", "web")), 1);
}

#[tokio::test]
async fn an_image_that_cannot_be_pulled_leaves_the_old_container_running_and_marks_the_plugin() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let out = app("app.out", "network.egress");
    m.install(&out, &granted(&out), super::langs())
        .await
        .unwrap();
    let web = container_name("app-out", "web");
    set_proxy(&m, Some("http://proxy:1"));
    docker.fail_pulls_except("gost", "toomanyrequests");
    m.apply_egress().await.unwrap();
    assert!(docker.containers().contains(&web));
    assert_eq!(starts(&docker, &web), 1);
    let PluginStatus::Degraded { reason } = status(&m, "app.out") else {
        panic!("应标为异常");
    };
    assert!(reason.contains("toomanyrequests"), "{reason}");
    assert!(!m.store.load(&id("app.out")).unwrap().unwrap().proxied);

    // 下次核对时再试。
    docker.fail_pulls(None);
    m.apply_egress().await.unwrap();
    assert_eq!(status(&m, "app.out"), PluginStatus::Running);
    assert_eq!(starts(&docker, &web), 2);
}

#[tokio::test]
async fn when_the_relay_cannot_start_no_plugin_is_touched() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let out = app("app.out", "network.egress");
    m.install(&out, &granted(&out), super::langs())
        .await
        .unwrap();
    set_proxy(&m, Some("http://proxy:1"));
    docker.fail_pulls(Some("toomanyrequests"));
    let err = m.apply_egress().await.unwrap_err();
    assert!(err.to_string().contains("toomanyrequests"), "{err}");
    assert_eq!(starts(&docker, &container_name("app-out", "web")), 1);
    assert_eq!(status(&m, "app.out"), PluginStatus::Running);
}

/// Bot 插件。`token` 是 compose 里用令牌的写法。
fn bot(services: &str) -> String {
    package(
        services,
        "  bot: {display_name: {zh: 回声, en: Echo}}\n  permissions: [matrix.bot, network.egress]",
    )
    .replace("id: app.test", "id: app.echo")
}

const IN_ENV: &str = "  bot:\n    image: echo\n    environment:\n      \
    TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}\n      AUTH: Bearer ${WONTON_MATRIX_ACCESS_TOKEN}";
const IN_COMMAND: &str =
    "  bot:\n    image: echo\n    command: [run, '${WONTON_MATRIX_ACCESS_TOKEN}']";

struct Bed {
    docker: FakeDocker,
    tuwunel: Fake,
    m: Arc<PluginManager>,
    _dir: tempfile::TempDir,
}

impl Bed {
    /// 没配代理时装好的 Bot 插件。
    async fn installed(services: &str) -> Self {
        let (docker, engine) = FakeDocker::start().await;
        let (tuwunel, config) = start().await;
        let dir = tempfile::tempdir().unwrap();
        let matrix = Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
            .await
            .unwrap();
        let m = manager_in_container(engine, &dir).with_matrix(Some(Arc::new(matrix)));
        let compose = bot(services);
        m.install(&compose, &granted(&compose), super::langs())
            .await
            .unwrap();
        Self {
            docker,
            tuwunel,
            m: Arc::new(m),
            _dir: dir,
        }
    }

    fn devices(&self) -> usize {
        self.tuwunel.state().tokens.len()
    }

    fn container(&self) -> String {
        container_name("app-echo", "bot")
    }

    fn reason(&self) -> String {
        match status(&self.m, "app.echo") {
            PluginStatus::Degraded { reason } => reason,
            other => panic!("应标为异常：{other:?}"),
        }
    }
}

#[tokio::test]
async fn a_bot_keeps_its_device_and_token_across_the_rebuild() {
    let bed = Bed::installed(IN_ENV).await;
    let web = bed.container();
    let token = env_of_container(&bed.docker, &web)["TOKEN"].clone();
    let devices = bed.devices();

    set_proxy(&bed.m, Some("http://proxy:1"));
    bed.m.apply_egress().await.unwrap();

    let env = env_of_container(&bed.docker, &web);
    assert_eq!(env["TOKEN"], token);
    assert_eq!(env["AUTH"], format!("Bearer {token}"));
    assert_eq!(env["HTTP_PROXY"], "http://egress.wonton.internal:3128");
    assert_eq!(starts(&bed.docker, &web), 2);
    assert_eq!(bed.devices(), devices, "不签新设备");
    assert_eq!(status(&bed.m, "app.echo"), PluginStatus::Running);
}

#[tokio::test]
async fn a_token_in_the_command_cannot_be_read_back_so_the_bot_is_left_as_it_is() {
    let bed = Bed::installed(IN_COMMAND).await;
    let web = bed.container();
    let devices = bed.devices();
    set_proxy(&bed.m, Some("http://proxy:1"));
    bed.m.apply_egress().await.unwrap();
    assert_eq!(starts(&bed.docker, &web), 1, "没删旧容器");
    assert!(!env_of_container(&bed.docker, &web).contains_key("HTTP_PROXY"));
    assert_eq!(bed.devices(), devices);
    let reason = bed.reason();
    let lang = CATALOG.negotiate(&["en"]);
    assert!(
        reason.contains(&CATALOG.format(
            lang,
            "plugin-settings-egress-token-unreadable",
            &[("service", "bot".into())],
        )),
        "{reason}"
    );
    assert!(
        reason.contains(&CATALOG.format(lang, "plugin-settings-egress-token-reinstall", &[],)),
        "{reason}"
    );
}

#[tokio::test]
async fn a_missing_old_container_is_reported_rather_than_signing_a_new_device() {
    let bed = Bed::installed(IN_ENV).await;
    let web = bed.container();
    let devices = bed.devices();
    bed.docker
        .add_stranger_container(&web, json!({"Env": ["OTHER=1"]}));
    set_proxy(&bed.m, Some("http://proxy:1"));
    bed.m.apply_egress().await.unwrap();
    assert_eq!(starts(&bed.docker, &web), 1);
    assert_eq!(bed.devices(), devices);
    let reason = bed.reason();
    assert!(
        reason.contains(&CATALOG.format(
            CATALOG.negotiate(&["en"]),
            "plugin-settings-egress-token-not-in-env",
            &[
                ("container", "wonton-app-echo--bot".into()),
                ("variable", "AUTH".into()),
            ],
        )),
        "{reason}"
    );
}
