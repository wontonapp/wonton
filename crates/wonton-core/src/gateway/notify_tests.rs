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

use super::fakes::loopback_tunnel;
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
        relay_dir: dir.path().join("relay"),
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
    let (tunnel, members) = loopback_tunnel(dir.path()).await;
    let gateway = Gateway::new(
        public,
        profile,
        super::tests::TOKEN.into(),
        members,
        plugins,
        None,
        HttpClient::new(),
        tunnel,
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
    /// 插件 `from` 发一条 JSON 通知（自己的格式）。
    async fn post(&self, from: &str, body: Value) -> (StatusCode, Value) {
        self.send(
            from,
            Method::POST,
            "/notices",
            "application/json",
            &[],
            body.to_string(),
        )
        .await
    }

    /// 按上游客户端的写法发：路径、方法、内容类型和头都由调用方给。
    async fn send(
        &self,
        from: &str,
        method: Method,
        path: &str,
        content_type: &str,
        headers: &[(&str, &str)],
        text: String,
    ) -> (StatusCode, Value) {
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "notify.wonton.internal:8080")
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CONTENT_LENGTH, text.len());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut request = request.body(Body::from(text)).unwrap();
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
    assert_eq!(status, StatusCode::OK);

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
    assert_eq!(status, StatusCode::OK);
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
        assert_eq!(status, StatusCode::OK, "{reply}");
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
    let long: String = "长".repeat(121);
    let (status, _) = home.post("172.30.1.5:40000", json!({"title": long})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let from = "172.30.1.5:40000";

    // 字段类型不对。
    let (status, reply) = home.post(from, json!({"title": 42})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("title"),
        "{reply}"
    );

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
    assert_eq!(status, StatusCode::OK);
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
    assert_eq!(status, StatusCode::OK);
    let sent = home.sent();
    let message = sent.last().unwrap();
    assert_eq!(message[NOTICE_FIELD]["path"], "/hosts/1");
}

#[tokio::test]
async fn a_webhook_payload_without_a_title_is_sent_as_readable_text() {
    let home = start().await;
    let payload = json!({
        "type": "chore_completed",
        "timestamp": "2026-11-12T09:30:00Z",
        "data": {"chore": {"name": "洗碗", "points": 2}, "completedBy": {"username": "alice"}},
    });
    let (status, reply) = home.post("172.30.1.5:40000", payload).await;
    assert_eq!(status, StatusCode::OK, "{reply}");

    let sent = home.sent();
    let [message] = sent.as_slice() else {
        panic!("只发了一条：{sent:?}");
    };
    // 没有 title 时从载荷里找一条当标题，整段拍平成可读文本。
    assert_eq!(message[NOTICE_FIELD]["title"], "洗碗");
    let text = message["body"].as_str().unwrap();
    assert!(text.contains("type: chore_completed"), "{text}");
    assert!(text.contains("data.chore.name: 洗碗"), "{text}");
    // 没写级别：info 只进聊天。
    assert_eq!(message["msgtype"], "m.notice");
}

#[tokio::test]
async fn an_ntfy_publish_uses_its_headers_and_plain_text_body() {
    let home = start().await;
    let (status, reply) = home
        .send(
            "172.30.1.5:40000",
            Method::PUT,
            "/backups",
            "text/plain",
            &[
                ("Title", "备份完成"),
                ("Priority", "4"),
                ("Tags", "backup,nightly"),
            ],
            "nas:/srv 已备份".to_owned(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{reply}");

    let sent = home.sent();
    let [message] = sent.as_slice() else {
        panic!("只发了一条：{sent:?}");
    };
    assert_eq!(message[NOTICE_FIELD]["title"], "备份完成");
    assert_eq!(message[NOTICE_FIELD]["level"], "warning");
    assert_eq!(message["msgtype"], "m.text");
    assert_eq!(
        message["body"],
        "监控：备份完成\nnas:/srv 已备份\n标签：backup、nightly"
    );
}

#[tokio::test]
async fn a_gotify_message_is_accepted() {
    let home = start().await;
    let payload = json!({"title": "磁盘快满", "message": "根分区剩 3%", "priority": 9});
    let (status, reply) = home
        .send(
            "172.30.2.5:40000",
            Method::POST,
            "/message?token=abc",
            "application/json",
            &[],
            payload.to_string(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{reply}");

    let sent = home.sent();
    let [message] = sent.as_slice() else {
        panic!("只发了一条：{sent:?}");
    };
    assert_eq!(message[NOTICE_FIELD]["title"], "磁盘快满");
    assert_eq!(message[NOTICE_FIELD]["body"], "根分区剩 3%");
    // Gotify 的 9 是“高优先”，要推送。
    assert_eq!(message[NOTICE_FIELD]["level"], "warning");
    assert_eq!(message["msgtype"], "m.text");
}

#[tokio::test]
async fn a_message_without_a_title_still_notifies() {
    let home = start().await;
    // 纯文本没有 Title 头：第一行当标题，其余是正文。
    let (status, _) = home
        .send(
            "172.30.1.5:40000",
            Method::POST,
            "/notices",
            "text/plain",
            &[],
            "证书到期\n换一张".to_owned(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // 空对象：标题兜底。
    let (status, _) = home.post("172.30.1.5:40000", json!({})).await;
    assert_eq!(status, StatusCode::OK);

    let sent = home.sent();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[0][NOTICE_FIELD]["title"], "证书到期");
    assert_eq!(sent[0][NOTICE_FIELD]["body"], "换一张");
    assert_eq!(sent[1][NOTICE_FIELD]["title"], "通知");
    assert!(sent[1][NOTICE_FIELD].get("body").is_none());

    // 只收 POST 和 PUT。
    let (status, reply) = home
        .send(
            "172.30.1.5:40000",
            Method::GET,
            "/notices",
            "text/plain",
            &[],
            String::new(),
        )
        .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{reply}");
}
