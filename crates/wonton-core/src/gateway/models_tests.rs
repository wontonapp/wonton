//! `models.wonton.internal`：只转发声明了 `models` 的插件的请求，流式响应到一块转一块，不缓冲。

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::routing::post;
use http_body_util::BodyExt;
use hyper::header;
use hyper::{Method, StatusCode};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::tests::Harness;
use crate::plugin::languages::Languages;
use crate::plugin::models::ModelsRoute;
use crate::plugin::{Permission, PluginPackage, PluginStatus};

const PROVIDER: &str = "services:\n  magpie:\n    image: magpie\nx-wonton:\n  spec: '0.4'\n  id: system.models\n  \
    version: 1.0.0\n  kind: system\n  title: {zh: 模型, en: Models}\n  description: {zh: 模型, en: Models}\n  \
    provides:\n    models:\n      service: magpie\n      port: 3425\n      apis: {openai: /v1}\n      \
    default_model: group/default\n      credentials:\n        issue: [issue, '{name}']\n        \
    revoke: [revoke, '{name}']\n  permissions: []";

/// 假网关：流式补全先回第一块，等测试放行再回第二块。
async fn streaming_gateway(release: oneshot::Receiver<()>) -> String {
    let release = Arc::new(Mutex::new(Some(release)));
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let release = release.lock().unwrap().take().expect("只请求一次");
            async move {
                let first = futures_util::stream::once(async {
                    Ok::<_, std::convert::Infallible>(Bytes::from("data: 1\n\n"))
                });
                let second = futures_util::stream::once(async move {
                    let _ = release.await;
                    Ok(Bytes::from("data: 2\n\n"))
                });
                Body::from_stream(futures_util::StreamExt::chain(first, second))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr.to_string()
}

async fn harness_with_gateway(upstream: String) -> Harness {
    harness_with_gateway_status(upstream, PluginStatus::Running).await
}

/// 同 [`harness_with_gateway`]，但网关是给定的状态（比如辅助容器坏了，插件已经是异常）。
async fn harness_with_gateway_status(upstream: String, status: PluginStatus) -> Harness {
    let h = Harness::start(None).await;
    let manifest = PluginPackage::parse(PROVIDER, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    let spec = manifest.provides.clone().unwrap().models.unwrap();
    let route = ModelsRoute {
        id: manifest.id.clone(),
        spec,
        upstream,
        credentials_container: "wonton-system-models--magpie".into(),
    };
    h.manager.register_for_test(manifest, status, None);
    h.manager.serve_models_for_test(route);
    h
}

fn chat(h: &Harness) -> hyper::Request<Body> {
    hyper::Request::builder()
        .method(Method::POST)
        .uri(format!("http://{}/v1/chat/completions", h.addr))
        .header(header::HOST, "models.wonton.internal:8080")
        .header(header::AUTHORIZATION, "Bearer key")
        .body(Body::from(r#"{"stream":true}"#))
        .unwrap()
}

#[tokio::test]
async fn model_requests_from_consumers_stream_through_the_core() {
    let (release, released) = oneshot::channel();
    let h = harness_with_gateway(streaming_gateway(released).await).await;
    // 测试的请求来自本机：把本机当成使用方的插件网络。
    h.manager
        .register_source_for_test("app.client", "127.0.0.0/8", vec![Permission::Models]);
    let response = h.http.send(chat(&h)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(first, "data: 1\n\n", "第一块不等整个响应结束就转过来");
    release.send(()).unwrap();
    let rest = body.collect().await.unwrap().to_bytes();
    assert_eq!(rest, "data: 2\n\n");
}

#[tokio::test]
async fn only_plugins_that_declare_models_reach_the_gateway() {
    let (_release, released) = oneshot::channel();
    let h = harness_with_gateway(streaming_gateway(released).await).await;
    let kind = |response: &hyper::Response<hyper::body::Incoming>| {
        let value = response.headers().get("x-wonton-error").cloned();
        value.map(|v| v.to_str().unwrap().to_owned())
    };
    let response = h.http.send(chat(&h)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(kind(&response).as_deref(), Some("unauthenticated"));
    h.manager
        .register_source_for_test("app.client", "127.0.0.0/8", Vec::new());
    let response = h.http.send(chat(&h)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(kind(&response).as_deref(), Some("permission_denied"));

    // 网关没在运行。
    let h = Harness::start(None).await;
    h.manager
        .register_source_for_test("app.client", "127.0.0.0/8", vec![Permission::Models]);
    let response = h.http.send(chat(&h)).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(kind(&response).as_deref(), Some("models_unavailable"));
}

/// 路由缺模型、服务商出错都只进详情页的轻提示（ADR-0055）：走安装 API，状态照常 running。
#[tokio::test]
async fn default_route_problems_never_degrade_the_installed_state() {
    for (listed, upstream) in [
        (false, StatusCode::OK),
        (true, StatusCode::UNAUTHORIZED),
        (true, StatusCode::OK),
    ] {
        let app = Router::new()
            .route(
                "/v1/models",
                axum::routing::get(move || async move {
                    let data = if listed {
                        serde_json::json!([{"id": "group/default"}])
                    } else {
                        serde_json::json!([])
                    };
                    (upstream, axum::Json(serde_json::json!({"data": data})))
                }),
            )
            .route(
                "/v1/chat/completions",
                post(move || async move {
                    (
                        upstream,
                        axum::Json(serde_json::json!(
                            {"choices": [{"message": {"role": "assistant", "content": "ok"}}]}
                        )),
                    )
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let h = harness_with_gateway(addr.to_string()).await;
        // 凭据命令在网关的容器里执行；假 Docker 里放进这个容器，exec 默认成功。
        h.docker
            .add_container("wonton-system-models--magpie", "system.models");
        let consumer = PROVIDER.split("x-wonton:").next().unwrap().to_owned()
            + "x-wonton:\n  spec: '0.6'\n  id: app.consumer\n  version: 1.0.0\n  kind: app\n  \
            title: {en: Consumer}\n  description: {en: Consumer}\n  permissions: [models]\n";
        let bearer = format!("Bearer {}", super::tests::TOKEN);
        let installed = h
            .send(
                Method::POST,
                super::tests::CORE,
                "/_wonton/api/plugins?grant=models",
                &[
                    ("authorization", &bearer),
                    ("content-type", "application/yaml"),
                ],
                Body::from(consumer),
            )
            .await;
        assert_eq!(
            installed.status,
            StatusCode::CREATED,
            "{}",
            installed.text()
        );
        assert_eq!(
            installed.json()["status"]["state"],
            "running",
            "listed={listed} upstream={upstream}"
        );
    }
}

// 网关服务自己好着、别的容器坏了：插件异常，模型请求照常转发（按服务判断）。
#[tokio::test]
async fn a_down_auxiliary_container_keeps_models_forwarding() {
    let (_release, released) = oneshot::channel();
    let degraded = PluginStatus::Degraded {
        reason: "worker: connection refused".into(),
    };
    let h = harness_with_gateway_status(streaming_gateway(released).await, degraded).await;
    h.manager
        .register_source_for_test("app.client", "127.0.0.0/8", vec![Permission::Models]);
    let response = h.http.send(chat(&h)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// 网关服务自己的容器不健康：路由不算能用，回 503 `models_unavailable`。
#[tokio::test]
async fn a_down_gateway_container_is_models_unavailable() {
    let (_release, released) = oneshot::channel();
    let h = harness_with_gateway(streaming_gateway(released).await).await;
    h.manager
        .register_source_for_test("app.client", "127.0.0.0/8", vec![Permission::Models]);
    h.manager.serving_for_test(
        &crate::plugin::PluginId::parse("system.models").unwrap(),
        &[],
    );
    let response = h.http.send(chat(&h)).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get("x-wonton-error")
            .unwrap()
            .to_str()
            .unwrap(),
        "models_unavailable"
    );
}
