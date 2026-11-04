//! 主人的身份交给插件的规则（ADR-0022）：和 `sso` 解耦，没配邮箱时如实失败。

use std::net::SocketAddr;

use axum::Router;
use axum::routing::get;

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::manager::PluginManager;
use super::super::manifest::Permission;
use super::super::package::PluginPackage;
use super::super::render::dry_run;
use super::egress::serve;
use super::manager::manager_with;
use super::{env_of_container, package};
use crate::config::Owner;

async fn web() -> SocketAddr {
    serve(Router::new().route("/", get(|| async { "ok" }))).await
}

fn without_email(engine: Engine, dir: &tempfile::TempDir) -> PluginManager {
    let mut manager = manager_with(engine, dir);
    manager.cfg.owner_email = None;
    manager
}

/// 一个 `sso: none`、申请了 `owner.identity` 的插件，环境变量 `MAIL` 写成 `mail`。
fn identity_app(mail: &str, port: u16) -> String {
    package(
        &format!(
            "  web:\n    image: nginx\n    environment:\n      ADMIN: ${{WONTON_OWNER_NAME}}\n      \
             MAIL: {mail}"
        ),
        &format!(
            "  webui: {{service: web, port: {port}, sso: none}}\n  \
             permissions: [owner.identity]"
        ),
    )
}

fn parsed(compose: &str) -> PluginPackage {
    PluginPackage::parse(compose).unwrap()
}

#[tokio::test]
async fn owner_identity_gives_an_sso_none_plugin_the_name_and_email() {
    let port = web().await.port();
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let compose = identity_app("${WONTON_OWNER_EMAIL:?}", port);
    let preview = m
        .preview_install(&parsed(&compose), &Languages::default())
        .await
        .unwrap();
    assert_eq!(preview.blocked, None);
    let line = preview
        .permissions
        .iter()
        .find(|p| p.permission == "owner.identity")
        .expect("权限列在安装页上");
    assert!(line.supported && line.risk.is_none());
    assert!(line.text.contains("username and email"), "{}", line.text);
    assert!(
        !preview.notices.iter().any(|n| n.text.contains("自动登录")),
        "应用自带登录，不会自动登录：{:?}",
        preview.notices
    );

    m.install(&compose, &[Permission::OwnerIdentity])
        .await
        .unwrap();
    let env = env_of_container(&docker, "wonton-app-test--web");
    assert_eq!(env["ADMIN"], "owner");
    assert_eq!(env["MAIL"], "owner@example.com");
}

#[test]
fn a_plugin_with_neither_sso_nor_the_permission_gets_no_identity() {
    let compose = package(
        "  web:\n    image: nginx\n    environment: {ADMIN: '${WONTON_OWNER_NAME}'}",
        "  webui: {service: web, port: 80}\n  permissions: []",
    );
    let error = dry_run(&parsed(&compose)).unwrap_err().to_string();
    assert!(error.contains("WONTON_OWNER_NAME"), "{error}");
    assert!(error.contains("没申请 owner.identity"), "{error}");
}

/// 声明了 `sso` 的插件，环境变量 `ADMIN` 引用主人的用户名。
fn sso_app(webui: &str) -> String {
    package(
        "  web:\n    image: nginx\n    environment: {ADMIN: '${WONTON_OWNER_NAME}'}",
        &format!("{webui}\n  permissions: []"),
    )
}

const OIDC: &str =
    "  webui:\n    service: web\n    port: 80\n    sso: oidc\n    oidc: {redirect_paths: [/cb]}";
const PROXY: &str = "  webui:\n    service: web\n    port: 80\n    sso: proxy\n    proxy:\n      \
                     headers: {Remote-User: '{user}'}";

#[test]
fn sso_does_not_bring_the_identity_variables() {
    for webui in [OIDC, PROXY] {
        let error = dry_run(&parsed(&sso_app(webui))).unwrap_err().to_string();
        assert!(error.contains("没申请 owner.identity"), "{error}");
    }
}

#[test]
fn proxy_sso_must_declare_a_header_or_a_cookie() {
    for webui in [
        "  webui: {service: web, port: 80, sso: proxy}",
        "  webui:\n    service: web\n    port: 80\n    sso: proxy\n    proxy: {headers: {}}",
    ] {
        let compose = package(
            "  web:\n    image: nginx",
            &format!("{webui}\n  permissions: []"),
        );
        let error = PluginPackage::parse(&compose).unwrap_err().to_string();
        assert!(error.contains("webui.proxy"), "{error}");
    }
}

#[tokio::test]
async fn the_preview_lists_the_identity_whenever_the_app_gets_it() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let secret_only = "  secrets:\n    key: {generate: {bytes: 32}}\n  webui:\n    service: web\n    port: 80\n    \
                       sso: proxy\n    proxy:\n      headers: {X-Key: '{secret:key}'}";
    for (webui, listed) in [(OIDC, true), (PROXY, true), (secret_only, false)] {
        let compose = package(
            "  web:\n    image: nginx",
            &format!("{webui}\n  permissions: []"),
        );
        let preview = m
            .preview_install(&parsed(&compose), &Languages::default())
            .await
            .unwrap();
        let shown = preview
            .notices
            .iter()
            .any(|n| n.text.contains("用户名和邮箱"));
        assert_eq!(shown, listed, "{webui}：{:?}", preview.notices);
    }
}

#[tokio::test]
async fn without_an_email_a_plugin_that_requires_it_does_not_install() {
    let port = web().await.port();
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = without_email(engine, &dir);
    let compose = identity_app("${WONTON_OWNER_EMAIL:?}", port);
    let preview = m
        .preview_install(&parsed(&compose), &Languages::default())
        .await
        .unwrap();
    let blocked = preview.blocked.expect("预览写明装不上");
    assert_eq!(blocked, crate::owner::NO_EMAIL);
    let error = m
        .install(&compose, &[Permission::OwnerIdentity])
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "unsupported", "{error}");
    assert!(
        error.to_string().contains(crate::owner::NO_EMAIL),
        "{error}"
    );
}

#[tokio::test]
async fn without_an_email_a_plugin_that_can_do_without_it_installs() {
    let port = web().await.port();
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = without_email(engine, &dir);
    let compose = identity_app("\"${WONTON_OWNER_EMAIL:-}\"", port);
    let preview = m
        .preview_install(&parsed(&compose), &Languages::default())
        .await
        .unwrap();
    assert_eq!(preview.blocked, None);
    m.install(&compose, &[Permission::OwnerIdentity])
        .await
        .unwrap();
    let env = env_of_container(&docker, "wonton-app-test--web");
    assert_eq!(env["ADMIN"], "owner");
    assert_eq!(env["MAIL"], "");
}

#[tokio::test]
async fn without_an_email_a_proxy_template_that_uses_it_does_not_install() {
    let port = web().await.port();
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = without_email(engine, &dir);
    let compose = package(
        "  web:\n    image: nginx",
        &format!(
            "  webui:\n    service: web\n    port: {port}\n    sso: proxy\n    proxy:\n      \
             headers: {{X-Mail: '{{email}}'}}\n  permissions: []"
        ),
    );
    let preview = m
        .preview_install(&parsed(&compose), &Languages::default())
        .await
        .unwrap();
    assert_eq!(preview.blocked.as_deref(), Some(crate::owner::NO_EMAIL));
    let error = m.install(&compose, &[]).await.unwrap_err();
    assert_eq!(error.kind(), "unsupported", "{error}");
    assert!(
        error.to_string().contains(crate::owner::NO_EMAIL),
        "{error}"
    );
}

#[tokio::test]
async fn a_proxy_template_stops_forwarding_when_the_email_is_removed_later() {
    let port = web().await.port();
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let compose = package(
        "  web:\n    image: nginx",
        &format!(
            "  webui:\n    service: web\n    port: {port}\n    sso: proxy\n    proxy:\n      \
             headers: {{X-Mail: '{{email}}'}}\n  permissions: []"
        ),
    );
    m.install(&compose, &[]).await.unwrap();
    let auth = m.route("app-test").unwrap().proxy.unwrap();
    let mut owner = Owner {
        id: "owner".into(),
        username: "owner".into(),
        display_name: "主人".into(),
        email: Some("me@example.com".into()),
    };
    assert_eq!(auth.headers(&owner).unwrap()[0].1, "me@example.com");
    owner.email = None;
    assert!(auth.headers(&owner).is_err(), "不代加一个空值");
}
