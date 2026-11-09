//! 插件整体异常时网关还转不转发网页：只看清单 `webui` 指向的那个容器。

use hyper::StatusCode;
use serde_json::{Value, json};

use super::tests::Harness;
use crate::plugin::PluginId;
use crate::plugin::PluginStatus;
use crate::plugin::languages::Languages;
use crate::plugin::names::container_name;

/// 装一个带网页服务和一个辅助容器的应用，网页指向测试里的回显服务；返回插件域名。
async fn install_web_and_worker(h: &Harness) -> String {
    let compose = format!(
        "services:\n  web:\n    image: nginx\n  worker:\n    image: redis\n\
         x-wonton:\n  spec: '0.5'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {{zh: 测试, en: Test}}\n  description: {{zh: 测试, en: Test}}\n  \
         webui: {{service: web, port: {}}}\n  permissions: []\n",
        h.upstream.port()
    );
    h.manager
        .install(&compose, &[], Languages::default())
        .await
        .unwrap();
    "app-test.wonton.localhost".to_owned()
}

fn unhealthy(output: &str, code: i64) -> Value {
    json!({
        "Status": "running", "Running": true,
        "Health": {"Status": "unhealthy", "Log": [{"ExitCode": code, "Output": output}]},
    })
}

fn degraded_reason(h: &Harness) -> String {
    let view = h
        .manager
        .list()
        .into_iter()
        .find(|view| view.id.to_string() == "app.test")
        .expect("应用装上了");
    match view.status {
        PluginStatus::Degraded { reason } => reason,
        other => panic!("{other:?}"),
    }
}

// 辅助容器坏了，网页服务自己好着：网关照常转发，插件整体仍是异常，原因里有坏掉的容器和它的探针输出。
#[tokio::test]
async fn a_healthy_web_page_is_served_while_an_auxiliary_container_is_unhealthy() {
    let h = Harness::start(None).await;
    let host = install_web_and_worker(&h).await;
    let worker = container_name("app-test", "worker");
    h.docker.set_state(
        &worker,
        unhealthy("curl: (7) Couldn't connect to server", 7),
    );
    h.manager.watch_containers().await;

    let reason = degraded_reason(&h);
    assert!(reason.contains(&worker), "{reason}");
    assert!(
        reason.contains("curl: (7) Couldn't connect to server"),
        "{reason}"
    );

    let app = h.app_session_on(&host).await;
    let page = h.get(&host, "/dashboard?id=1", &[("cookie", &app)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.json()["path"], "/dashboard?id=1");
}

// 网页服务自己不健康：照旧 503。原因进详情，正文不再写 Rust 的调试格式。
#[tokio::test]
async fn an_unhealthy_web_container_still_answers_not_ready() {
    let h = Harness::start(None).await;
    let host = install_web_and_worker(&h).await;
    h.docker.set_state(
        &container_name("app-test", "web"),
        unhealthy("wget: connection refused", 1),
    );
    h.manager.watch_containers().await;

    let app = h.app_session_on(&host).await;
    let page = h
        .get(&host, "/", &[("accept", "text/html"), ("cookie", &app)])
        .await;
    assert_eq!(page.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(page.error_kind(), Some("not_ready"));
    assert!(page.text().contains("<details>"), "{}", page.text());
    assert!(
        page.text().contains("wget: connection refused"),
        "{}",
        page.text()
    );
    assert!(!page.text().contains("Degraded"), "{}", page.text());

    // 接口拿到的是 JSON，同一段原因放进 `detail`。
    let api = h
        .get(&host, "/api/data", &[("accept", "*/*"), ("cookie", &app)])
        .await;
    assert_eq!(api.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(api.error_kind(), Some("not_ready"));
    let error = &api.json()["error"];
    assert_eq!(error["category"], "not_ready");
    assert!(
        error["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("wget: connection refused")),
        "{}",
        api.text()
    );
}

/// 装一个 `sso: proxy` 的网页加辅助容器，网页指向测试里的回显服务。返回插件域名和插件的网页密钥。
async fn install_signed_in_web_and_worker(h: &Harness) -> (String, String) {
    let compose = format!(
        "services:\n  web:\n    image: nginx\n  worker:\n    image: redis\n\
         x-wonton:\n  spec: '0.5'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {{zh: 测试, en: Test}}\n  description: {{zh: 测试, en: Test}}\n  \
         webui:\n    service: web\n    port: {}\n    sso: proxy\n    \
         proxy:\n      headers: {{Remote-User: '{{user}}'}}\n      cookies: {{vault_web: '{{secret:web_key}}'}}\n  \
         secrets:\n    web_key: {{generate: {{bytes: 32}}}}\n  permissions: []\n",
        h.upstream.port()
    );
    h.manager
        .install(&compose, &[], Languages::default())
        .await
        .unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let secrets = h.manager.store().secrets().load(&id).unwrap().unwrap();
    let key = String::from_utf8(secrets["web_key"].clone()).unwrap();
    ("app-test.wonton.localhost".to_owned(), key)
}

// 核心重启后核对：辅助容器坏着，网页服务自己好着，网关照常转发，插件如实标为异常。
#[tokio::test]
async fn a_restart_keeps_serving_the_web_page_while_an_auxiliary_container_is_down() {
    let h = Harness::start(None).await;
    let host = install_web_and_worker(&h).await;
    let worker = container_name("app-test", "worker");
    h.docker
        .set_state(&worker, unhealthy("curl: (7) Couldn't connect", 7));
    h.manager.restore().await.unwrap();

    let reason = degraded_reason(&h);
    assert!(reason.contains(&worker), "{reason}");

    let app = h.app_session_on(&host).await;
    let page = h.get(&host, "/dashboard?id=1", &[("cookie", &app)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.json()["path"], "/dashboard?id=1");
}

// 重启后的部分部署里 `sso: proxy` 的代登录凭据照常带出：503 不会变成应用自己的登录错误。
#[tokio::test]
async fn a_restart_keeps_the_proxy_sign_in_credentials() {
    let h = Harness::start(None).await;
    let (host, key) = install_signed_in_web_and_worker(&h).await;
    h.docker.set_state(
        &container_name("app-test", "worker"),
        unhealthy("redis: connection refused", 1),
    );
    h.manager.restore().await.unwrap();

    let app = h.app_session_on(&host).await;
    let page = h.get(&host, "/", &[("cookie", &app)]).await;
    assert_eq!(page.status, StatusCode::OK);
    let headers = &page.json()["headers"];
    assert_eq!(headers["remote-user"], "owner");
    assert_eq!(headers["cookie"], format!("vault_web={key}"));
}

// 重启时网页服务自己不健康：照样 503，原因进详情。
#[tokio::test]
async fn a_restart_with_an_unhealthy_web_container_answers_not_ready() {
    let h = Harness::start(None).await;
    let host = install_web_and_worker(&h).await;
    h.docker.set_state(
        &container_name("app-test", "web"),
        unhealthy("wget: connection refused", 1),
    );
    h.manager.restore().await.unwrap();

    let app = h.app_session_on(&host).await;
    let page = h
        .get(&host, "/", &[("accept", "text/html"), ("cookie", &app)])
        .await;
    assert_eq!(page.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(page.error_kind(), Some("not_ready"));
    assert!(
        page.text().contains("wget: connection refused"),
        "{}",
        page.text()
    );
}

// 网页容器整个不见了：核对的重建会把它建回来（见 plugin::tests::render_drift）；重建再失败（起不来）
// 才是"没有地址"的稳定状态：一样是 503 而不是 404 `no_webui`。
#[tokio::test]
async fn a_restart_with_a_missing_web_container_answers_not_ready() {
    let h = Harness::start(None).await;
    let host = install_web_and_worker(&h).await;
    h.docker.remove(&container_name("app-test", "web"));
    h.docker.fail_starts(Some("容器起不来"));
    h.manager.restore().await.unwrap();

    let app = h.app_session_on(&host).await;
    let page = h
        .get(&host, "/", &[("accept", "text/html"), ("cookie", &app)])
        .await;
    assert_eq!(page.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(page.error_kind(), Some("not_ready"));
}
