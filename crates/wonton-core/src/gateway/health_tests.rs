//! `plugins.health` 的契约：按来源鉴权、列表、当场探测、限频（规范“插件健康探测”）。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use hyper::{Method, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::tests::Harness;
use crate::plugin::languages::Languages;
use crate::plugin::{Permission, PluginPackage, PluginStatus};

const HEALTH: &str = "health.wonton.internal";
const MONITOR: &str = "com.example.monitor";
const MONITOR_SUBNET: &str = "127.0.0.0/8";
const PROBE_ID: &str = "app.example.probe";
const DOWN_ID: &str = "app.example.down";

const PROBE_MANIFEST: &str = "name: probe
services:
  web:
    image: example/probe
x-wonton:
  spec: '0.4'
  id: app.example.probe
  version: 0.1.0
  kind: app
  title: {zh: 探测, en: Probe}
  description: {zh: 健康探测的目标, en: probe target}
  webui: {service: web, port: 8080}
  permissions: []
";

const DOWN_MANIFEST: &str = "name: down
services:
  web:
    image: example/down
x-wonton:
  spec: '0.4'
  id: app.example.down
  version: 0.1.0
  kind: app
  title: {zh: 停了, en: Down}
  description: {zh: 已停止的目标, en: stopped target}
  webui: {service: web, port: 8080}
  permissions: []
";

/// 记录收到的 Host 头，回 200：核对探测请求带的是插件的公开地址。
async fn host_recorder(seen: Arc<Mutex<Vec<String>>>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                if let Some(host) = request.lines().find_map(|line| {
                    line.strip_prefix("host: ")
                        .or_else(|| line.strip_prefix("Host: "))
                }) {
                    seen.lock().unwrap().push(host.trim().to_owned());
                }
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                    .await;
            });
        }
    });
    addr
}

fn register_monitor(harness: &Harness, permissions: Vec<Permission>) {
    harness
        .manager
        .register_source_for_test(MONITOR, MONITOR_SUBNET, permissions);
}

fn register_plugin(harness: &Harness, manifest: &str, status: PluginStatus, upstream: &str) {
    let manifest = PluginPackage::parse(manifest, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    harness
        .manager
        .register_for_test(manifest, status, Some(upstream.to_owned()));
}

async fn start() -> Harness {
    let harness = Harness::start(None).await;
    harness.manager.register_source_for_test(
        MONITOR,
        MONITOR_SUBNET,
        vec![Permission::PluginsHealth],
    );
    harness
}

#[tokio::test]
async fn without_the_permission_the_health_host_is_forbidden() {
    let harness = Harness::start(None).await;
    // 来源插件网络认得出来，但没申请 plugins.health。
    register_monitor(&harness, vec![]);
    let reply = harness.get(HEALTH, "/plugins", &[]).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), Some("permission_denied"));
    // 不是插件网络里的来源也一样。
    let harness = Harness::start(None).await;
    let reply = harness.get(HEALTH, "/plugins", &[]).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), Some("unauthenticated"));
}

#[tokio::test]
async fn granted_plugins_list_every_installed_plugin() {
    let harness = start().await;
    let reply = harness.get(HEALTH, "/plugins", &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    let plugins = reply.json().as_array().expect("列表是 JSON 数组").clone();
    let whoami = plugins
        .iter()
        .find(|p| p["id"] == "app.whoami")
        .expect("装着的回显插件在列表里");
    assert_eq!(whoami["state"], "running");
    assert_eq!(whoami["title"]["zh"], "回显");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let checked_at = whoami["checked_at"].as_u64().expect("checked_at 是秒数");
    assert!(
        checked_at <= now && now - checked_at < 3600,
        "{checked_at} 不是最近的核对时间"
    );
    // 列表不探测：每个条目只有状态，没有 probe。
    for plugin in &plugins {
        assert!(plugin.get("probe").is_none(), "{plugin} 不该带 probe");
    }
}

#[tokio::test]
async fn detail_probes_the_webui_with_the_public_host() {
    let harness = start().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = host_recorder(seen.clone()).await;
    register_plugin(
        &harness,
        PROBE_MANIFEST,
        PluginStatus::Running,
        &recorder.to_string(),
    );
    let reply = harness
        .get(HEALTH, &format!("/plugins/{PROBE_ID}"), &[])
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let report = reply.json();
    assert_eq!(report["state"], "running");
    assert_eq!(report["probe"]["reachable"], true);
    assert_eq!(report["probe"]["http_status"], 200);
    assert!(report["probe"]["latency_ms"].as_u64().is_some());
    assert!(report.get("error_kind").is_none());
    assert!(report.get("probe_reason").is_none());
    let hosts = seen.lock().unwrap().clone();
    assert_eq!(hosts, vec!["app-example-probe.wonton.localhost".to_owned()]);
}

#[tokio::test]
async fn stopped_target_reports_unreachable_and_honest_state() {
    let harness = start().await;
    register_plugin(
        &harness,
        DOWN_MANIFEST,
        PluginStatus::Stopped,
        "127.0.0.1:1",
    );
    let reply = harness
        .get(HEALTH, &format!("/plugins/{DOWN_ID}"), &[])
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let report = reply.json();
    assert_eq!(report["state"], "stopped");
    assert_eq!(report["probe"]["reachable"], false);
    assert_eq!(report["probe"]["error_kind"], "unreachable");
    assert!(report["probe"].get("http_status").is_none());
    assert!(report["probe"]["latency_ms"].as_u64().is_some());
}

#[tokio::test]
async fn erroring_upstream_reports_upstream_error_with_status() {
    let harness = start().await;
    let broken = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = broken.local_addr().unwrap();
    tokio::spawn(async move {
        // 只收连接、回 503，别的什么都不做。
        while let Ok((mut stream, _)) = broken.accept().await {
            let _ = stream
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n")
                .await;
        }
    });
    register_plugin(
        &harness,
        PROBE_MANIFEST,
        PluginStatus::Running,
        &addr.to_string(),
    );
    let reply = harness
        .get(HEALTH, &format!("/plugins/{PROBE_ID}"), &[])
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let report = reply.json();
    assert_eq!(report["probe"]["reachable"], false);
    assert_eq!(report["probe"]["error_kind"], "upstream_error");
    assert_eq!(report["probe"]["http_status"], 503);
}

#[tokio::test]
async fn unknown_plugin_and_other_methods_are_rejected() {
    let harness = start().await;
    let reply = harness
        .get(HEALTH, "/plugins/app.example.missing", &[])
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_kind(), Some("not_installed"));
    let reply = harness.get(HEALTH, "/plugins", &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    let reply = harness
        .send(Method::POST, HEALTH, "/plugins", &[], Body::from("{}"))
        .await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn five_requests_per_second_then_429() {
    let harness = start().await;
    for _ in 0..5 {
        assert_eq!(
            harness.get(HEALTH, "/plugins", &[]).await.status,
            StatusCode::OK
        );
    }
    let reply = harness.get(HEALTH, "/plugins", &[]).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(reply.error_kind(), Some("too_many_requests"));
    // 过了一秒额度补回来。
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(
        harness.get(HEALTH, "/plugins", &[]).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn responses_carry_no_secret_fields() {
    let harness = start().await;
    register_plugin(
        &harness,
        PROBE_MANIFEST,
        PluginStatus::Running,
        "127.0.0.1:1",
    );
    let list = harness.get(HEALTH, "/plugins", &[]).await;
    let detail = harness
        .get(HEALTH, &format!("/plugins/{PROBE_ID}"), &[])
        .await;
    for reply in [list, detail] {
        assert_eq!(reply.status, StatusCode::OK);
        let text = reply.text().to_ascii_lowercase();
        for secret in [
            "secret",
            "token",
            "password",
            "credential",
            "private_key",
            "api_key",
        ] {
            assert!(!text.contains(secret), "响应里出现了 {secret}：{text}");
        }
    }
}
