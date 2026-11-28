//! 网关代登录（sso: proxy）：请求头、Cookie 密钥和身份占位符的规则。

use axum::Router;
use axum::routing::get;

use super::super::super::PluginPackage;
use super::super::super::fake_docker::FakeDocker;
use super::super::super::package::PackageError;
use super::super::super::render::dry_run;
use super::super::egress::serve;
use super::super::env_of_container;
use super::super::{langs, package};
use super::manager_with;

/// 网关代登录的插件：代加一个请求头和一个 Cookie，Cookie 的值是核心生成的密钥。
fn proxy_app(port: u16) -> String {
    package(
        "  web:\n    image: nginx",
        &format!(
            "  webui:\n    service: web\n    port: {port}\n    sso: proxy\n    proxy:\n      \
             headers: {{Remote-User: '{{user}}'}}\n      cookies: {{app_web: '{{secret:web_key}}'}}\n  \
             secrets: {{web_key: {{generate: {{bytes: 32}}}}}}\n  permissions: []"
        ),
    )
}

#[tokio::test]
async fn proxy_sso_plugins_get_their_credentials_from_their_own_secrets() {
    // 假 Docker 把插件容器的地址报成 127.0.0.1，安装最后一步检查的网页就是这个本机服务。
    let web = serve(Router::new().route("/", get(|| async { "ok" }))).await;
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine.clone(), &dir);
    let compose = proxy_app(web.port());
    m.install(&compose, &[], langs()).await.unwrap();
    let owner = crate::owner::Owner {
        username: "owner".into(),
        display_name: "主人".into(),
        email: Some("owner@wonton.localhost".into()),
    };
    let auth = m.route("app-test", "/").unwrap().proxy.unwrap();
    let header = auth.headers(&owner, true).unwrap();
    assert_eq!(
        (header[0].0.as_str(), header[0].1.as_str()),
        ("remote-user", "owner")
    );
    let cookies = auth.cookies(&owner, true).unwrap();
    let (name, key) = (&cookies[0].0, cookies[0].1.clone());
    assert_eq!((*name, key.len()), ("app_web", 64), "32 字节的十六进制");
    assert_eq!(auth.secrets(), [key.as_str()]);
    // 核心重启后从密钥库读回同一个值。
    let restarted = manager_with(engine, &dir);
    restarted.restore().await.unwrap();
    let auth = restarted.route("app-test", "/").unwrap().proxy.unwrap();
    assert_eq!(auth.cookies(&owner, true).unwrap()[0].1, key);
}

#[tokio::test]
async fn sso_plugins_that_ask_for_the_identity_get_the_owner_name_and_email() {
    let web = serve(Router::new().route("/", get(|| async { "ok" }))).await;
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let compose = package(
        "  web:\n    image: nginx\n    environment:\n      \
         ADMIN: ${WONTON_OWNER_NAME}\n      MAIL: ${WONTON_OWNER_EMAIL}",
        &format!(
            "  webui:\n    service: web\n    port: {}\n    sso: proxy\n    proxy:\n      \
             headers: {{Remote-User: '{{user}}'}}\n  permissions: [owner.identity]",
            web.port()
        ),
    );
    m.install(&compose, &["owner.identity".parse().unwrap()], langs())
        .await
        .unwrap();
    let env = env_of_container(&docker, "wonton-app-test--web");
    assert_eq!(env["ADMIN"], "owner");
    assert_eq!(env["MAIL"], "owner@wonton.internal");
}

#[test]
fn plugins_without_sso_cannot_reference_the_owner() {
    let compose = package(
        "  web:\n    image: nginx\n    environment: {MAIL: '${WONTON_OWNER_EMAIL}'}",
        "  webui: {service: web, port: 80}\n  permissions: []",
    );
    let pkg = PluginPackage::parse(&compose, &langs()).unwrap();
    let err = dry_run(&pkg, &langs()).unwrap_err().to_string();
    assert!(err.contains("WONTON_OWNER_EMAIL"), "{err}");
}

#[test]
fn proxy_sso_templates_use_known_placeholders_and_long_generated_secrets() {
    let parse = |headers: &str, cookies: &str, secrets: &str| {
        let manifest = format!(
            "  webui:\n    service: web\n    port: 80\n    sso: proxy\n    proxy:\n      \
             headers: {headers}\n      cookies: {cookies}\n  secrets: {secrets}\n  permissions: []"
        );
        PluginPackage::parse(&package("  web:\n    image: nginx", &manifest), &langs())
    };
    let key = "{web_key: {generate: {bytes: 32}}}";
    let ok = parse(
        "{Remote-User: '{user} <{email}>', Remote-Role: '{role}'}",
        "{app_web: 'v1.{secret:web_key}', app_role: '{role}'}",
        key,
    );
    assert!(ok.is_ok(), "{ok:?}");
    for (headers, cookies, secrets) in [
        // 不认识的占位符、不配对的括号。
        ("{X-A: '{who}'}", "{}", key),
        ("{X-A: '{user'}", "{}", key),
        ("{X-A: 'user}'}", "{}", key),
        // 只能引用核心生成、至少 16 字节的密钥。
        ("{}", "{a: '{secret:nope}'}", key),
        (
            "{}",
            "{a: '{secret:web_key}'}",
            "{web_key: {generate: {bytes: 8}}}",
        ),
        (
            "{}",
            "{a: '{secret:web_key}'}",
            "{web_key: {collect: {service: web, path: /k}}}",
        ),
        // Cookie 的值里不能放用户名这类可能带分隔符的字段，也不能有空白；请求头的值里不能有控制字符。
        ("{}", "{a: '{name}'}", key),
        ("{}", "{a: 'v {secret:web_key}'}", key),
        ("{X-A: \"a\\u0001b\"}", "{}", key),
        // 网关自己管的头、逐跳头和保留的 Cookie 名。
        ("{Host: x}", "{}", key),
        ("{X-Forwarded-For: x}", "{}", key),
        ("{Cookie: x}", "{}", key),
        ("{}", "{wonton_app: x}", key),
        ("{}", "{'a;b': x}", key),
    ] {
        assert!(
            matches!(
                parse(headers, cookies, secrets),
                Err(PackageError::Manifest(_))
            ),
            "{headers} {cookies} {secrets}"
        );
    }
}
