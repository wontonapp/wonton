//! 插件整体异常时网关还转不转发网页：只看清单 `webui` 指向的那个容器。

use hyper::StatusCode;
use serde_json::{Value, json};

use super::tests::Harness;
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
