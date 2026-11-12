//! `notify.wonton.internal`：只放行申请了 `notify` 的插件；字段校验；发进通知房间的内容和级别对应的 msgtype；
//! 频率超了 429 且同一段超限期间只提示主人一次（ADR-0047）。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use http_body_util::BodyExt;
use hyper::{Method, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_protocol::NOTICE_FIELD;

use super::tunnel_tests::loopback_tunnel;
use super::{Gateway, router};
use crate::http::HttpClient;
use crate::matrix::{Matrix, NoticeRoom};
use crate::owner::OwnerStore;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{Permission, PluginManager, PluginPackage, PluginStatus};

/// 申请了 `notify`、没有网页的插件。
const MONITOR: &str = "app.example.monitor";
/// 申请了 `notify`、有网页的插件。
const PANEL: &str = "app.example.panel";

fn manifest(id: &str, webui: bool) -> String {
    let webui = if webui {
        "  webui: {service: web, port: 8080}\n"
    } else {
        ""
    };
    format!(
        "name: notes\nservices:\n  web:\n    image: example/web\nx-wonton:\n  spec: '0.4'\n  id: {id}\n  \
         version: 0.1.0\n  kind: app\n  title: {{zh: 监控, en: Monitor}}\n  description: {{zh: 监控, en: Monitor}}\n  \
         permissions: [notify]\n{webui}"
    )
}

struct Home {
    app: axum::Router,
    /// 假 Tuwunel：发出去的消息都在 `sent` 里。
    fake: crate::matrix::fake::Fake,
    _dir: tempfile::TempDir,
}

/// 核心接着假的 Tuwunel；两个插件的来源地址在登记的网段里。
async fn start() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let (fake, matrix_config) = crate::matrix::fake::start().await;
    let matrix = Arc::new(
        Matrix::connect(&matrix_config, dir.path(), HttpClient::new())
            .await
            .unwrap(),
    );
    let public = crate::config::PublicUrls {
        scheme: "http".into(),
        base_domain: super::tests::CORE.into(),
        port: 18480,
    };
    let profile = OwnerStore::for_test(dir.path(), &owner());
    let lifecycle = LifecycleConfig {
        public: public.clone(),
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: profile.clone(),
        gateway_port: 8080,
        plugin_slice: None,
        instance: Default::default(),
    };
    let plugins = Arc::new(PluginManager::new(
        Engine::unreachable_for_test(),
        Store::open(dir.path().join("plugins")).unwrap(),
        HttpClient::new(),
        None,
        lifecycle,
        ImageWorker::in_process(),
    ));
    for (id, webui, subnet) in [
        (MONITOR, false, "172.30.1.0/24"),
        (PANEL, true, "172.30.2.0/24"),
    ] {
        let package = PluginPackage::parse(
            &manifest(id, webui),
            &Languages::from_header(Some("zh-Hans")),
        )
        .unwrap();
        plugins.register_for_test(package.manifest, PluginStatus::Running, None);
        plugins.register_source_for_test(id, subnet, vec![Permission::Notify]);
    }
    let notices = Arc::new(NoticeRoom::new(
        matrix,
        "owner",
        dir.path().join("notice-room"),
        Arc::new(crate::plugin::languages::Languages::default),
    ));
    let gateway = Gateway::new(
        public,
        profile,
        super::tests::TOKEN.into(),
        plugins,
        None,
        HttpClient::new(),
        loopback_tunnel(dir.path()).await,
    )
    .with_notices(Some(notices));
    Home {
        app: router(Arc::new(gateway)),
        fake,
        _dir: dir,
    }
}

fn owner() -> crate::owner::Owner {
    crate::owner::Owner {
        username: "owner".into(),
        display_name: "主人".into(),
        email: Some("owner@example.com".into()),
    }
}

impl Home {
    /// 插件 `from` 发一条通知。
    async fn post(&self, from: &str, body: Value) -> (StatusCode, Value) {
        let text = body.to_string();
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri("/notices")
            .header(header::HOST, "notify.wonton.internal:8080")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, text.len())
            .body(Body::from(text))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(from.parse::<SocketAddr>().unwrap()));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// 发进通知房间的消息。
    fn sent(&self) -> Vec<Value> {
        self.fake
            .state()
            .sent
            .iter()
            .map(|(_, content)| content.clone())
            .collect()
    }

    /// 核心自己发的提示（不是插件通知）。
    fn hints(&self) -> Vec<Value> {
        self.sent()
            .into_iter()
            .filter(|message| message.get(NOTICE_FIELD).is_none())
            .collect()
    }
}

#[tokio::test]
async fn a_notice_reaches_the_owner_with_level_and_structured_fields() {
    let home = start().await;
    let body = json!({"title": "下载完成", "body": "Ubuntu 24.04.iso", "level": "info"});
    let (status, _) = home.post("172.30.1.5:40000", body).await;
    assert_eq!(status, StatusCode::CREATED);

    let sent = home.sent();
    let [message] = sent.as_slice() else {
        panic!("只发了一条：{sent:?}");
    };
    // info 是 m.notice，不推送。
    assert_eq!(message["msgtype"], "m.notice");
    assert_eq!(message["body"], "监控：下载完成\nUbuntu 24.04.iso");
    assert_eq!(message[NOTICE_FIELD]["plugin_id"], MONITOR);
    assert_eq!(message[NOTICE_FIELD]["plugin_name"], "监控");
    assert_eq!(message[NOTICE_FIELD]["level"], "info");
    assert_eq!(message[NOTICE_FIELD]["title"], "下载完成");
    assert_eq!(message[NOTICE_FIELD]["body"], "Ubuntu 24.04.iso");

    // warning、critical 是 m.text，推送。
    let (status, _) = home
        .post(
            "172.30.1.5:40000",
            json!({"title": "CPU 100%", "level": "critical"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let sent = home.sent();
    let message = sent.last().unwrap();
    assert_eq!(message["msgtype"], "m.text");
    assert_eq!(message["body"], "监控：CPU 100%", "没有正文时就是一行");
    assert_eq!(message[NOTICE_FIELD]["level"], "critical");
    assert!(message[NOTICE_FIELD].get("body").is_none());
}

#[tokio::test]
async fn only_plugins_with_the_permission_and_from_their_network_may_send() {
    let home = start().await;
    // 没申请 notify 的插件（本测试没登记别的权限的来源，用一个不在插件网络里的地址冒充）。
    let (status, reply) = home
        .post("10.9.9.9:40000", json!({"title": "偷偷的"}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{reply}");
    assert_eq!(reply["error"]["kind"], "unauthenticated");
    assert!(home.sent().is_empty());
}

#[tokio::test]
async fn the_eleventh_within_a_minute_is_denied_and_the_owner_is_told_once() {
    let home = start().await;
    for i in 0..10 {
        let (status, reply) = home
            .post("172.30.1.5:40000", json!({"title": format!("第 {i} 条")}))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{reply}");
    }
    let (status, reply) = home
        .post("172.30.1.5:40000", json!({"title": "第 11 条"}))
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{reply}");
    assert_eq!(reply["error"]["kind"], "rate_limited");
    // 再超的丢弃，不再提示。
    let (status, _) = home
        .post("172.30.1.5:40000", json!({"title": "第 12 条"}))
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    let notices = home
        .sent()
        .into_iter()
        .filter(|m| m.get(NOTICE_FIELD).is_some())
        .count();
    assert_eq!(notices, 10, "第 11 条被丢弃");
    let hints = home.hints();
    assert_eq!(hints.len(), 1, "只提示一次：{hints:?}");
    assert_eq!(hints[0]["msgtype"], "m.notice");
    assert!(
        hints[0]["body"].as_str().unwrap().contains("监控"),
        "{hints:?}"
    );
}

#[tokio::test]
async fn invalid_fields_are_rejected_with_the_field_named() {
    let home = start().await;
    let from = "172.30.1.5:40000";
    let (status, reply) = home.post(from, json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert_eq!(reply["error"]["kind"], "invalid_input");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("title"),
        "{reply}"
    );

    let long: String = "长".repeat(121);
    let (status, _) = home.post(from, json!({"title": long})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = home
        .post(from, json!({"title": "急", "level": "emergency"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = home
        .post(from, json!({"title": "急", "path": "hosts/1"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 没有网页的插件不能带 path。
    let (status, reply) = home
        .post(from, json!({"title": "急", "path": "/hosts/1"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(
        reply["error"]["message"].as_str().unwrap().contains("path"),
        "{reply}"
    );

    // 有网页的插件可以，正文超长也不行。
    let (status, _) = home
        .post(
            "172.30.2.5:40000",
            json!({"title": "急", "path": "/hosts/1"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let long: String = "长".repeat(2001);
    let (status, _) = home.post(from, json!({"title": "急", "body": long})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 请求体上限 16 KiB。
    let big = "x".repeat(20 * 1024);
    let (status, reply) = home
        .post(from, json!({"title": "急", "padding": big}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(
        reply["error"]["message"].as_str().unwrap().contains("KiB"),
        "{reply}"
    );
}

#[tokio::test]
async fn a_notice_with_a_path_keeps_it_for_the_app_to_open() {
    let home = start().await;
    let (status, _) = home
        .post(
            "172.30.2.5:40000",
            json!({"title": "磁盘快满了", "path": "/hosts/1"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let sent = home.sent();
    let message = sent.last().unwrap();
    assert_eq!(message[NOTICE_FIELD]["path"], "/hosts/1");
}
