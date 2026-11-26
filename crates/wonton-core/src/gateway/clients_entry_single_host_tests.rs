//! `WONTON_EXTERNAL_URL` 和 `single_host`（ADR-0089 第 5 节、第 8 节 `external_url_contract`）：
//! 三条路看到的 `Host`、`X-Forwarded-*` 相同，换 DNS 服务商不变，总开关不改值也不重建。

use super::*;

const SINGLE_APP: &str = r#"
services:
  app:
    image: example/app:1
    environment:
      URL: ${WONTON_EXTERNAL_URL}
      WONTON_EXTERNAL_URL: ${WONTON_EXTERNAL_URL}
  side:
    image: example/app:1
x-wonton:
  spec: '0.14'
  id: io.wonton.test.single
  version: 1.0.0
  kind: app
  title: { zh: 单地址示例, en: Single host example }
  description:
    zh: 规范对外地址的行为测试用
    en: For external URL behaviour tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      single_host: true
      auth:
        app:
          setup_done: [sh, /wonton/setup-done.sh]
  webui:
    service: app
    port: 8080
    sso: proxy
    proxy:
      headers:
        X-Remote-User: "{user}"
"#;

const SINGLE_ID: &str = "io.wonton.test.single";
const CANONICAL: &str = "single.home.example.test";

fn single() -> crate::plugin::PluginId {
    crate::plugin::PluginId::parse(SINGLE_ID).unwrap()
}

/// 登记单地址插件；给两个服务的镜像都放行，重建时不用拉镜像。
fn register(home: &Home) {
    home.register(SINGLE_APP, "single", true, true);
    let instance = crate::plugin::names::Instance::default();
    for service in ["app", "side"] {
        home.docker.add_image_container(
            &instance.container("io.wonton.test.single", service),
            "example/app:1",
        );
    }
}

/// 只改设置里的 DNS 服务商，不签证书。
fn set_provider(home: &Home, provider: &str) {
    let store = serde_json::json!({
        "suffix": "home.example.test",
        "provider": provider,
        "credentials": {"TOKEN": "x"},
        "terms_accepted": true,
    });
    std::fs::write(home.install.join("entry/domain.json"), store.to_string()).unwrap();
}

async fn single_value(home: &Home) -> String {
    let record = home.manager.record_for_test(&single()).unwrap();
    crate::clients::external_url::for_record(&home.gateway, &record)
        .await
        .unwrap()
}

/// 三条路各请求一次，返回（上游收到的头，客户端发出去的 Host）。
async fn three_ways(home: &Home) -> Vec<(Received, String)> {
    let before = home.upstream_hits().len();
    let public_host = home.host("single");
    let public = home.get(&public_host, "/", &entry_public()).await;
    assert_eq!(public.status, StatusCode::OK, "{}", public.text());
    let lan_host = format!("{}:8483", home.lan_host("single"));
    let lan = home.get(&lan_host, "/", &entry_lan()).await;
    assert_eq!(lan.status, StatusCode::OK, "{}", lan.text());
    let host = format!("{}.{}", single().slug(), home.gateway.public.base_domain);
    let cookie = session(home, &host).await;
    let tunnel_host = format!("{host}:{}", home.gateway.public.port);
    let tunnel = home
        .main_request(Method::GET, &host, "/", &[("cookie", cookie.as_str())], "")
        .await;
    assert_eq!(tunnel.status, StatusCode::OK, "{}", tunnel.text());
    home.upstream_hits()[before..]
        .iter()
        .cloned()
        .zip([public_host, lan_host, tunnel_host])
        .collect()
}

/// 登录核心、换交接码、在插件域名兑换会话。
async fn session(home: &Home, host: &str) -> String {
    let form = format!("token={}", crate::gateway::tests::TOKEN);
    let login = home
        .main_request(
            Method::POST,
            home.gateway.public.base_domain.as_str(),
            "/_wonton/login",
            &[("content-type", "application/x-www-form-urlencoded")],
            &form,
        )
        .await;
    assert_eq!(login.status, StatusCode::SEE_OTHER, "{}", login.text());
    let core = login.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let to = format!("http://{host}:{}/hello", home.gateway.public.port);
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &to)
        .finish();
    let handoff = home
        .main_request(
            Method::GET,
            home.gateway.public.base_domain.as_str(),
            &format!("/_wonton/sso?{query}"),
            &[("cookie", &core)],
            "",
        )
        .await;
    assert_eq!(handoff.status, StatusCode::SEE_OTHER);
    let target = url::Url::parse(&handoff.location()).unwrap();
    let redeem = home
        .main_request(
            Method::GET,
            host,
            &format!("{}?{}", target.path(), target.query().unwrap()),
            &[],
            "",
        )
        .await;
    assert_eq!(redeem.status, StatusCode::SEE_OTHER);
    redeem
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|value| {
            let value = value.to_str().unwrap();
            value
                .starts_with(crate::gateway::APP_COOKIE)
                .then(|| value.split(';').next().unwrap().to_owned())
        })
        .unwrap()
}

#[tokio::test]
async fn external_url_contract() {
    let home = Home::start().await;
    register(&home);
    let mut seen = Vec::new();
    for provider in ["desec", "cloudflare"] {
        set_provider(&home, provider);
        assert_eq!(
            single_value(&home).await,
            format!("https://{CANONICAL}"),
            "{provider}"
        );
        let hits = three_ways(&home).await;
        assert_eq!(hits.len(), 3);
        for (hit, sent_host) in &hits {
            assert_eq!(hit.value("host"), Some(CANONICAL), "{provider}");
            assert_eq!(hit.value("x-forwarded-proto"), Some("https"), "{provider}");
            assert_eq!(
                hit.value("x-forwarded-host"),
                Some(sent_host.as_str()),
                "{provider}：原来的 Host 要原样留在 X-Forwarded-Host"
            );
        }
        seen.push(
            hits.iter()
                .map(|(hit, _)| {
                    (
                        hit.value("host").map(str::to_owned),
                        hit.value("x-forwarded-proto").map(str::to_owned),
                        hit.value("x-forwarded-host").map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(seen[0], seen[1], "换 DNS 服务商不改值");
}

#[tokio::test]
async fn host_kept_without_single_host() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    let host = home.host("entryapp");
    let reply = home.get(&host, "/", &entry_public()).await;
    assert_eq!(reply.status, StatusCode::OK);
    let hit = home.upstream_hits().remove(0);
    assert_eq!(hit.value("host"), Some(host.as_str()));
    assert_eq!(hit.value("x-forwarded-proto"), Some("https"));
}

#[tokio::test]
async fn canonical_host_uses_the_entry_label_not_the_slug() {
    let home = Home::start().await;
    home.register(SINGLE_APP, "single-2", true, true);
    set_provider(&home, "desec");
    let host = home.host("single-2");
    let reply = home.get(&host, "/", &entry_public()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let hit = home.upstream_hits().remove(0);
    assert_eq!(hit.value("host"), Some("single-2.home.example.test"));
    assert_eq!(hit.value("x-forwarded-host"), Some(host.as_str()));
}

#[tokio::test]
async fn failed_rebuild_rolls_back_switch_entry_and_env() {
    let home = Home::start().await;
    let id = single();
    let instance = crate::plugin::names::Instance::default();
    let app = instance.container(&id.slug(), "app");
    // 记录全关：按全关登记；先成功开关一轮，让 app 容器带上一份按“全关”渲染的 env。
    home.register(SINGLE_APP, "single", false, false);
    assert_eq!(
        home.switch(SINGLE_ID, "public", true).await.status,
        StatusCode::OK
    );
    assert_eq!(
        home.switch(SINGLE_ID, "public", false).await.status,
        StatusCode::OK
    );
    let before_env = home.docker.env(&app);
    home.docker.fail_next_create(&app, "boom");
    let reply = home.switch(SINGLE_ID, "public", true).await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert_eq!(reply.error_kind(), Some("plugin_unavailable"));
    let record = home.manager.record_for_test(&id).unwrap();
    assert!(!record.clients_entries.public && !record.clients_entries.lan);
    let denied = home.get(&home.host("single"), "/", &entry_public()).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(home.docker.env(&app), before_env);
}

#[tokio::test]
async fn value_change_rebuilds_only_referencing_services() {
    let home = Home::start().await;
    register(&home);
    let id = single();
    let instance = crate::plugin::names::Instance::default();
    let app = instance.container(&id.slug(), "app");
    let side = instance.container(&id.slug(), "side");
    // 记录先全关，由开关接口打开。
    home.manager
        .clients_entry_for_test(&id, "single", false, false);

    let reply = home.switch(SINGLE_ID, "public", true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["effect"], "restart");
    assert_eq!(home.docker.creations(&app), 1);
    assert_eq!(home.docker.creations(&side), 0);
    let env = home.docker.env(&app);
    assert!(
        env.iter()
            .any(|item| item == &format!("WONTON_EXTERNAL_URL=https://{CANONICAL}")),
        "{env:?}"
    );

    let reply = home.switch(SINGLE_ID, "public", false).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(home.docker.creations(&app), 2);
    assert_eq!(home.docker.creations(&side), 0);

    let reply = home.switch(SINGLE_ID, "public", true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(home.docker.creations(&app), 3);
    assert_eq!(home.docker.creations(&side), 0);
}

#[tokio::test]
async fn public_access_switch_does_not_rebuild() {
    let home = Home::start().await;
    register(&home);
    let id = single();
    let instance = crate::plugin::names::Instance::default();
    let app = instance.container(&id.slug(), "app");
    home.manager
        .clients_entry_for_test(&id, "single", false, false);
    assert_eq!(
        home.switch(SINGLE_ID, "public", true).await.status,
        StatusCode::OK
    );
    let value = single_value(&home).await;
    let creations = home.docker.creations(&app);
    home.gateway
        .public_access
        .store(false, std::sync::atomic::Ordering::Relaxed);
    crate::clients::apply::reconcile(&home.gateway)
        .await
        .unwrap();
    assert_eq!(single_value(&home).await, value);
    assert_eq!(
        home.docker.creations(&app),
        creations,
        "总开关不改值也不重建"
    );
}
