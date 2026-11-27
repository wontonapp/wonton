//! `WONTON_EXTERNAL_URL` 只注入给开得出入口的插件（ADR-0089 第 5 节）。

use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::manifest::Permission;
use super::super::package::PluginPackage;
use super::super::render::render;
use super::manager::manager_with;

fn langs() -> Languages {
    Languages::default()
}

const SERVED: &str = r#"
services:
  app:
    image: nginx
    environment:
      URL: ${WONTON_EXTERNAL_URL}
      WONTON_EXTERNAL_URL: ${WONTON_EXTERNAL_URL}
x-wonton:
  spec: '0.14'
  id: app.test
  version: 1.0.0
  kind: app
  title: { zh: 测试, en: Test }
  description: { zh: 测试, en: Test }
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        app:
          setup_done: ['true']
  webui:
    service: app
    port: 8080
    sso: proxy
    proxy:
      headers:
        X-Remote-User: "{user}"
"#;

const PLAIN: &str = r#"
services:
  app:
    image: nginx
x-wonton:
  spec: '0.14'
  id: app.test
  version: 1.0.0
  kind: app
  title: { zh: 测试, en: Test }
  description: { zh: 测试, en: Test }
  permissions: []
  webui:
    service: app
    port: 8080
"#;

#[tokio::test]
async fn external_url_injected_only_when_declared() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);

    let served = PluginPackage::parse(SERVED, &langs()).unwrap();
    let ctx = m
        .install_context(
            &served.manifest,
            None,
            Default::default(),
            &[Permission::NetworkClients],
            &langs(),
        )
        .await
        .unwrap();
    let value = ctx.external_url.clone().expect("声明了入口就有值");
    assert_eq!(
        value,
        ctx.app_url.clone().unwrap(),
        "入口都没开时等于 App 地址"
    );
    let plan = render(&served, &ctx, &langs()).unwrap();
    let container = plan
        .containers
        .iter()
        .find(|container| container.service == "app")
        .unwrap();
    let env = container.body.env.clone().unwrap();
    assert!(
        env.iter()
            .any(|item| item == &format!("WONTON_EXTERNAL_URL={value}")),
        "{env:?}"
    );

    let plain = PluginPackage::parse(PLAIN, &langs()).unwrap();
    let ctx = m
        .install_context(&plain.manifest, None, Default::default(), &[], &langs())
        .await
        .unwrap();
    assert!(ctx.external_url.is_none());
    let plan = render(&plain, &ctx, &langs()).unwrap();
    let container = plan
        .containers
        .iter()
        .find(|container| container.service == "app")
        .unwrap();
    let env = container.body.env.clone().unwrap_or_default();
    assert!(!env.iter().any(|item| item.starts_with("URL=")), "{env:?}");
}
