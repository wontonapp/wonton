//! 活动的存活期由真实 TCP 请求与响应体驱动，不以收到响应头为结束。

use std::collections::BTreeMap;
use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{Request, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use wonton_protocol::BotActivityReport;

use crate::gateway::tests::Harness;
use crate::matrix::BotAccount;
use crate::plugin::bots::{BotRecord, BotStore};
use crate::plugin::languages::Languages;
use crate::plugin::models::ModelsRoute;
use crate::plugin::{Permission, PluginPackage, PluginStatus};

const PROVIDER: &str = "services:\n  model:\n    image: model\nx-wonton:\n  spec: '0.4'\n  id: system.models\n  \
    version: 1.0.0\n  kind: system\n  title: {zh: 模型, en: Models}\n  description: {zh: 模型, en: Models}\n  \
    provides:\n    models:\n      service: model\n      port: 3425\n      apis: {openai: /v1}\n      \
    default_model: group/default\n      credentials:\n        issue: [issue, '{name}']\n        \
    revoke: [revoke, '{name}']\n  permissions: []";

struct Reply {
    headers: oneshot::Sender<()>,
    finish: oneshot::Sender<bool>,
}

async fn upstream() -> (String, mpsc::UnboundedReceiver<Reply>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if socket.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    request.push(byte[0]);
                }
                let (headers, ready) = oneshot::channel();
                let (finish, finished) = oneshot::channel();
                if tx.send(Reply { headers, finish }).is_err() || ready.await.is_err() {
                    return;
                }
                if socket
                    .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/event-stream\r\nX-Model: test\r\n\r\n9\r\ndata: 1\n\n\r\n")
                    .await
                    .is_err()
                {
                    return;
                }
                if finished.await == Ok(true) {
                    let _ = socket.write_all(b"9\r\ndata: 2\n\n\r\n0\r\n\r\n").await;
                }
            });
        }
    });
    (addr, rx)
}

fn langs() -> Languages {
    Languages::from_header(Some("zh-Hans"))
}

fn route(h: &Harness, upstream: String) {
    let manifest = PluginPackage::parse(PROVIDER, &langs()).unwrap().manifest;
    let spec = manifest.provides.clone().unwrap().models.unwrap();
    h.manager
        .register_for_test(manifest.clone(), PluginStatus::Running, None);
    h.manager.serve_models_for_test(ModelsRoute {
        id: manifest.id,
        spec,
        upstream,
        credentials_container: "wonton-system-models--model".into(),
    });
}

fn bot(h: &Harness, id: &str) {
    let yaml = format!(
        "services:\n  bot:\n    image: bot\nx-wonton:\n  spec: '0.4'\n  id: {id}\n  \
         version: 1.0.0\n  kind: app\n  title: {{zh: 测试, en: Test}}\n  \
         description: {{zh: 测试, en: Test}}\n  permissions: [matrix.bot, models]"
    );
    let manifest = PluginPackage::parse(&yaml, &langs()).unwrap().manifest;
    BotStore::new(h.data_dir().join("plugins/bots"))
        .save(
            &manifest.id,
            &BotRecord {
                account: BotAccount {
                    user_id: format!("@{id}:wonton.localhost"),
                    home_room: format!("!{id}:wonton.localhost"),
                },
                device_id: Some("BOT_DEVICE".into()),
            },
        )
        .unwrap();
    h.manager
        .register_for_test(manifest, PluginStatus::Running, None);
}

async fn harness(addr: String) -> Harness {
    let h = Harness::start(None).await;
    route(&h, addr);
    bot(&h, "app.bot");
    bot(&h, "app.other");
    h.manager.register_source_for_test(
        "app.bot",
        "127.0.0.0/8",
        vec![Permission::Models, Permission::MatrixBot],
    );
    h
}

fn chat(h: &Harness) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("http://{}/v1/chat/completions", h.addr))
        .header("host", "models.wonton.internal:8080")
        .body(Body::empty())
        .unwrap()
}

fn report(h: &Harness) -> BotActivityReport {
    let facts = h.manager.bot_activity_facts().unwrap();
    h.manager.activity.report(facts, &BTreeMap::new())
}

fn assert_count(h: &Harness, expected: u32) {
    let report = report(h);
    assert_eq!(report.bots.len(), 2);
    for bot in report.bots {
        assert_eq!(bot.user_id, format!("@{}:wonton.localhost", bot.plugin_id));
        assert_eq!(
            bot.model_requests,
            if bot.plugin_id == "app.bot" {
                expected
            } else {
                0
            }
        );
        assert_eq!((bot.tool_calls, bot.waiting), (0, 0));
    }
}

async fn wait_count(h: &Harness, expected: u32) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if report(h)
                .bots
                .iter()
                .find(|bot| bot.plugin_id == "app.bot")
                .unwrap()
                .model_requests
                == expected
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("活动计数没有收敛");
    assert_count(h, expected);
}

async fn first(body: &mut hyper::body::Incoming) {
    let data = tokio::time::timeout(Duration::from_secs(3), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    assert_eq!(data, "data: 1\n\n");
}

#[tokio::test]
async fn model_activity_covers_headers_and_stream_until_eof() {
    let (addr, mut requests) = upstream().await;
    let h = harness(addr).await;
    assert_count(&h, 0);
    let before = report(&h).rev;
    let http = h.http.clone();
    let request = chat(&h);
    let sending = tokio::spawn(async move { http.send(request).await.unwrap() });
    let reply = requests.recv().await.unwrap();
    assert_count(&h, 1);
    assert!(report(&h).rev > before);
    reply.headers.send(()).unwrap();
    let response = sending.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-model"], "test");
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();
    first(&mut body).await;
    assert_count(&h, 1);
    reply.finish.send(true).unwrap();
    assert_eq!(body.collect().await.unwrap().to_bytes(), "data: 2\n\n");
    wait_count(&h, 0).await;
}

#[tokio::test]
async fn downstream_disconnect_releases_model_activity() {
    let (addr, mut requests) = upstream().await;
    let h = harness(addr).await;
    let http = h.http.clone();
    let request = chat(&h);
    let sending = tokio::spawn(async move { http.send(request).await.unwrap() });
    let reply = requests.recv().await.unwrap();
    reply.headers.send(()).unwrap();
    let mut body = sending.await.unwrap().into_body();
    first(&mut body).await;
    assert_count(&h, 1);
    drop(body);
    wait_count(&h, 0).await;
    let _ = reply.finish.send(true);
}

#[tokio::test]
async fn concurrent_models_and_forward_errors_keep_independent_guards() {
    let (addr, mut requests) = upstream().await;
    let h = harness(addr).await;
    let mut streams = Vec::new();
    for _ in 0..2 {
        let http = h.http.clone();
        let request = chat(&h);
        let sending = tokio::spawn(async move { http.send(request).await.unwrap() });
        let reply = requests.recv().await.unwrap();
        reply.headers.send(()).unwrap();
        let mut body = sending.await.unwrap().into_body();
        first(&mut body).await;
        streams.push((body, reply.finish));
    }
    assert_count(&h, 2);
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = closed.local_addr().unwrap().to_string();
    drop(closed);
    route(&h, addr);
    let response = h.http.send(chat(&h)).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["x-wonton-error"], "upstream_unreachable");
    assert_count(&h, 2);
    let (body, finish) = streams.pop().unwrap();
    finish.send(false).unwrap();
    assert!(body.collect().await.is_err(), "截断上游响应必须向下游报错");
    wait_count(&h, 1).await;
    let (body, finish) = streams.pop().unwrap();
    finish.send(true).unwrap();
    assert_eq!(body.collect().await.unwrap().to_bytes(), "data: 2\n\n");
    wait_count(&h, 0).await;
}
