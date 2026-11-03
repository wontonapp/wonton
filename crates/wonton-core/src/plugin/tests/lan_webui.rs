//! 开关 `lan_webui`（ADR-0023）：能冒充主人或者抢先注册的局域网端口默认不开，主人看过风险后打开；改开关时重建开端口的
//! 容器。别的局域网端口不受影响。

use axum::Router;
use axum::routing::get;
use serde_json::Value;
use wonton_protocol::{LanPort, LanProtocol};

use super::super::fake_docker::FakeDocker;
use super::super::lan::webui::{open_ports, switched};
use super::super::languages::Languages;
use super::super::names::container_name;
use super::super::{PluginId, PluginManager, PluginPackage, PluginStatus};
use super::egress::serve;
use super::lan::{binding, created, granted, office, on};
use super::{env_of_container, package};

const IDENTITY: &str = "sso: proxy, proxy: {headers: {Remote-User: '{user}'}}";
const OWN_LOGIN: &str = "sso: none";

/// `web` 是网页，环境变量写在 `env`；`lan.ports` 列网页的端口，`extra` 再加别的。
fn app(port: u16, webui: &str, env: &str, extra: &str) -> String {
    package(
        &format!(
            "  web:\n    image: nginx\n    environment: {{ADDRESSES: '${{WONTON_LAN_ADDRESSES}}', {env}}}\n  \
             smb:\n    image: example/samba"
        ),
        &format!(
            "  webui: {{service: web, port: {port}, {webui}}}\n  \
             lan:\n    ports: [{{service: web, port: {port}}}{extra}]\n  \
             permissions: [network.lan]"
        ),
    )
}

async fn web_port() -> u16 {
    serve(Router::new().route("/", get(|| async { "ok" })))
        .await
        .port()
}

fn parsed(compose: &str) -> PluginPackage {
    PluginPackage::parse(compose).unwrap()
}

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}

fn published(docker: &FakeDocker, service: &str) -> Value {
    created(docker, service)["HostConfig"]["PortBindings"].clone()
}

fn ports_of(package: &PluginPackage) -> Vec<u16> {
    let ports = switched(package).map(|s| s.ports).unwrap_or_default();
    ports.iter().map(|p| p.port).collect()
}

#[test]
fn identity_headers_switch_the_whole_service_unless_it_trusts_only_the_core() {
    let more = ", {service: web, port: 139}, {service: smb, port: 445}";
    let wide = parsed(&app(80, IDENTITY, "TRUST: 0.0.0.0/0", more));
    assert_eq!(ports_of(&wide), [80, 139]);
    assert_eq!(open_ports(&wide, false).len(), 1, "别的服务的端口照开");
    assert_eq!(open_ports(&wide, true).len(), 3);

    let unset = parsed(&app(80, IDENTITY, "A: b", ""));
    assert_eq!(ports_of(&unset), [80], "没写信任的代理地址也算");
    let only_core = parsed(&app(80, IDENTITY, "TRUST: '${WONTON_CORE_ADDRESS}/32'", ""));
    assert_eq!(ports_of(&only_core), [0u16; 0]);
    let widened = parsed(&app(
        80,
        IDENTITY,
        "TRUST: '${WONTON_CORE_ADDRESS}/32,::/0'",
        "",
    ));
    assert_eq!(ports_of(&widened), [80], "作者放宽了");
}

#[test]
fn apps_with_their_own_login_switch_only_the_webui_port() {
    let own = parsed(&app(80, OWN_LOGIN, "A: b", ", {service: web, port: 139}"));
    assert_eq!(ports_of(&own), [80]);
    assert_eq!(open_ports(&own, false).len(), 1);
}

#[test]
fn logins_the_lan_cannot_borrow_need_no_switch() {
    let secret = "sso: proxy, proxy: {headers: {X-Key: '{secret:key}'}}";
    let compose = app(80, secret, "A: b", "").replace(
        "  permissions:",
        "  secrets:\n    key: {generate: {bytes: 32}}\n  permissions:",
    );
    assert_eq!(switched(&parsed(&compose)), None);
    assert_eq!(open_ports(&parsed(&compose), false).len(), 1);
}

#[tokio::test]
async fn closed_ports_are_not_published_and_need_no_address() {
    let (docker, m, detected, _dir) = on(office()).await;
    *detected.lock().unwrap() = Err("读不了网卡".into());
    let port = web_port().await;
    let compose = app(port, OWN_LOGIN, "A: b", "");

    let preview = m
        .preview_install(&parsed(&compose), &Languages::default())
        .await
        .unwrap();
    assert_eq!(preview.blocked, None);
    let offer = preview.lan_webui.expect("有开关");
    assert!(!offer.enabled);
    let tcp = LanPort {
        port,
        protocol: LanProtocol::Tcp,
    };
    assert_eq!(offer.ports, [tcp]);

    m.install(&compose, &granted(&compose)).await.unwrap();
    assert_eq!(published(&docker, "web"), Value::Null);
    assert_eq!(docker.network("wonton-app-test--lan"), None);
    let endpoints = created(&docker, "web")["NetworkingConfig"]["EndpointsConfig"].clone();
    assert_eq!(endpoints.as_object().unwrap().len(), 1, "不接 --lan 网络");
    let env = env_of_container(&docker, "wonton-app-test--web");
    assert_eq!(env["ADDRESSES"], "");
    assert!(m.lan_plugins().is_empty(), "没有开着的端口，不用检测");
    let detail = m.installed_detail(&id(), &Languages::default()).unwrap();
    assert!(!detail.lan_webui.unwrap().enabled);
}

#[tokio::test]
async fn opening_at_install_publishes_the_port_and_survives_restart() {
    let (docker, m, _detected, dir) = on(office()).await;
    let port = web_port().await;
    let compose = app(port, OWN_LOGIN, "A: b", "");
    let package = parsed(&compose);
    let granted = package.manifest.permissions.clone();
    m.install_user(package, &granted, true).await.unwrap();

    let key = format!("{port}/tcp");
    assert_eq!(
        published(&docker, "web")[&key],
        binding("192.168.0.193", &port.to_string())
    );
    let env = env_of_container(&docker, "wonton-app-test--web");
    assert_eq!(env["ADDRESSES"], "192.168.0.193");
    assert_eq!(m.lan_plugins().len(), 1);

    let record = super::super::store::Store::open(dir.path().join("plugins"))
        .unwrap()
        .load(&id())
        .unwrap()
        .unwrap();
    assert!(record.lan_webui);
}

#[tokio::test]
async fn switching_rebuilds_the_opening_containers_only() {
    let (docker, m, _detected, _dir) = on(office()).await;
    let port = web_port().await;
    let compose = app(port, OWN_LOGIN, "A: b", ", {service: smb, port: 445}");
    m.install(&compose, &granted(&compose)).await.unwrap();
    assert_eq!(published(&docker, "web"), Value::Null);
    let key = format!("{port}/tcp");

    let detail = m
        .set_lan_webui(id(), true, Languages::default())
        .await
        .unwrap();
    assert!(detail.lan_webui.unwrap().enabled);
    let binding_on = binding("192.168.0.193", &port.to_string());
    assert_eq!(published(&docker, "web")[&key], binding_on);
    assert_eq!(m.lan_plugins()[0].1.ports.len(), 2);
    assert_eq!(status(&m), PluginStatus::Running);

    // 再设成一样的值：什么都不做。
    let before = docker.writes().len();
    m.set_lan_webui(id(), true, Languages::default())
        .await
        .unwrap();
    assert_eq!(docker.writes().len(), before);

    let detail = m
        .set_lan_webui(id(), false, Languages::default())
        .await
        .unwrap();
    assert!(!detail.lan_webui.unwrap().enabled);
    assert_eq!(published(&docker, "web"), Value::Null);
    assert_eq!(
        published(&docker, "smb")["445/tcp"],
        binding("192.168.0.193", "445"),
        "不用开关的端口一直开着"
    );
    assert_eq!(m.lan_plugins()[0].1.ports.len(), 1);
}

#[tokio::test]
async fn opening_reports_why_it_failed_and_keeps_the_switch_off() {
    let (docker, m, detected, _dir) = on(office()).await;
    let port = web_port().await;
    let compose = app(port, OWN_LOGIN, "A: b", "");
    m.install(&compose, &granted(&compose)).await.unwrap();

    // 选不出地址：直接报错，容器不动。
    *detected.lock().unwrap() = Err("读不了网卡".into());
    let before = docker.writes().len();
    let err = m
        .set_lan_webui(id(), true, Languages::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, crate::plugin::InstallError::LanUnavailable(_)),
        "{err:?}"
    );
    assert_eq!(docker.writes().len(), before);
    *detected.lock().unwrap() = Ok(office());

    // 端口被别的插件开着：冲突。
    let other = package(
        "  smb:\n    image: example/samba",
        &format!(
            "  lan: {{ports: [{{service: smb, port: {port}}}]}}\n  permissions: [network.lan]"
        ),
    )
    .replace("app.test", "app.other");
    m.install(&other, &granted(&other)).await.unwrap();
    let err = m
        .set_lan_webui(id(), true, Languages::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, crate::plugin::InstallError::Conflict(_)),
        "{err:?}"
    );
    m.uninstall(&PluginId::parse("app.other").unwrap(), false)
        .await
        .unwrap();

    // 重建失败：报错，开关退回去；拉不到镜像时退回去也失败，插件标为异常，再试一次能恢复。
    docker.fail_pulls(Some("拉不到镜像"));
    let err = m
        .set_lan_webui(id(), true, Languages::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("拉不到镜像"), "{err}");
    let detail = m.installed_detail(&id(), &Languages::default()).unwrap();
    assert!(!detail.lan_webui.unwrap().enabled);
    assert!(matches!(status(&m), PluginStatus::Degraded { .. }));
    docker.fail_pulls(None);
    m.set_lan_webui(id(), true, Languages::default())
        .await
        .unwrap();
    assert_eq!(status(&m), PluginStatus::Running);
}

#[tokio::test]
async fn closing_never_reopens_the_port_when_it_fails() {
    let (docker, m, _detected, _dir) = on(office()).await;
    let port = web_port().await;
    let compose = app(port, OWN_LOGIN, "A: b", "");
    m.install(&compose, &granted(&compose)).await.unwrap();
    m.set_lan_webui(id(), true, Languages::default())
        .await
        .unwrap();
    let enabled = |m: &PluginManager| {
        let detail = m.installed_detail(&id(), &Languages::default()).unwrap();
        detail.lan_webui.unwrap().enabled
    };

    // 别的服务不健康：端口关了，报错，定期检测也不开回去；那个服务好了就恢复，网页不再重建。
    let smb = container_name("app-test", "smb");
    let health = |status| serde_json::json!({"Status": "running", "Running": true, "Health": {"Status": status}});
    docker.set_state(&smb, health("unhealthy"));
    m.set_lan_webui(id(), false, Languages::default())
        .await
        .unwrap_err();
    assert!(!enabled(&m));
    assert!(matches!(status(&m), PluginStatus::Degraded { .. }));
    let closed = docker.writes().len();
    m.lan_pass().await.unwrap();
    assert_eq!(published(&docker, "web"), Value::Null);
    assert!(!enabled(&m));
    assert!(matches!(status(&m), PluginStatus::Degraded { .. }));
    docker.set_state(&smb, health("healthy"));
    m.lan_pass().await.unwrap();
    assert_eq!(status(&m), PluginStatus::Running);
    assert_eq!(published(&docker, "web"), Value::Null);
    assert!(
        !docker.writes()[closed..].iter().any(|w| w.contains("web")),
        "端口已经关了，网页不重建"
    );

    // 重建失败，端口还开着：定期检测接着关。
    m.set_lan_webui(id(), true, Languages::default())
        .await
        .unwrap();
    docker.fail_pulls(Some("拉不到镜像"));
    m.set_lan_webui(id(), false, Languages::default())
        .await
        .unwrap_err();
    assert!(!enabled(&m));
    docker.fail_pulls(None);
    m.lan_pass().await.unwrap();
    assert_eq!(status(&m), PluginStatus::Running);
    assert_eq!(published(&docker, "web"), Value::Null);
}

#[tokio::test]
async fn closed_ports_do_not_conflict_with_other_plugins() {
    let (_docker, m, _detected, _dir) = on(office()).await;
    let port = web_port().await;
    let compose = app(port, OWN_LOGIN, "A: b", "");
    m.install(&compose, &granted(&compose)).await.unwrap();
    let other = package(
        "  smb:\n    image: example/samba",
        &format!(
            "  lan: {{ports: [{{service: smb, port: {port}}}]}}\n  permissions: [network.lan]"
        ),
    )
    .replace("app.test", "app.other");
    m.install(&other, &granted(&other)).await.unwrap();
}

#[tokio::test]
async fn plugins_without_the_switch_refuse_it() {
    let (_docker, m, _detected, _dir) = on(office()).await;
    let compose = package(
        "  smb:\n    image: example/samba",
        "  lan: {ports: [{service: smb, port: 445}]}\n  permissions: [network.lan]",
    );
    m.install(&compose, &granted(&compose)).await.unwrap();
    let err = m
        .set_lan_webui(id(), true, Languages::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, crate::plugin::InstallError::Invalid(_)),
        "{err:?}"
    );
    let detail = m.installed_detail(&id(), &Languages::default()).unwrap();
    assert_eq!(detail.lan_webui, None);
}

fn status(m: &PluginManager) -> PluginStatus {
    m.list().into_iter().next().expect("装了插件").status
}
