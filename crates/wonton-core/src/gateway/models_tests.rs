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
    let h = Harness::start(None).await;
    let manifest = PluginPackage::parse(PROVIDER).unwrap().manifest;
    let spec = manifest.provides.clone().unwrap().models.unwrap();
    let route = ModelsRoute {
        id: manifest.id.clone(),
        spec,
        upstream,
        credentials_container: "wonton-system-models--magpie".into(),
    };
    h.manager
        .register_for_test(manifest, PluginStatus::Running, None);
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
