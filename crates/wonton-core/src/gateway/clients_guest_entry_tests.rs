//! 访客链接的入口和接口行为：只给声明了的插件建、有效期和条数上限、Cookie 交换、路径范围、
//! 身份头清洗、收回和过期断连接。

use std::time::Duration;

use hyper::header;
use hyper::{Method, StatusCode};

use super::*;
use crate::clients::guests::COOKIE_NAME;
use crate::plugin::PluginId;
use wonton_protocol::{GUEST_LINKS_MAX, GUEST_TTL_MAX};

/// 只声明能接待访客、没有 `clients.http`。
const GUEST_ONLY: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.14'
  id: io.wonton.test.guests
  version: 1.0.0
  kind: app
  title: { zh: 访客示例, en: Guest example }
  description:
    zh: 访客链接的行为测试用
    en: For guest link behaviour tests
  permissions: [network.clients]
  webui:
    service: app
    port: 8080
    guests: true
"#;

/// 有自己的入口密码，也能接待访客。
const GUEST_AND_HTTP: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.14'
  id: io.wonton.test.guesthttp
  version: 1.0.0
  kind: app
  title: { zh: 访客加登录示例, en: Guest and sign-in example }
  description:
    zh: 访客链接越过入口密码的行为测试用
    en: For guest links over the entry password
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        gateway: {}
  webui:
    service: app
    port: 8080
    guests: true
    sso: proxy
    proxy:
      headers:
        X-Remote-User: "{user}"
"#;

impl Home {
    /// 登记一个能接待访客的插件，开关和标签照常写进记录。
    fn register_guests(&self, compose: &str, label: &str, lan: bool, public: bool) -> PluginId {
        let langs = crate::plugin::languages::Languages::from_header(None);
        let manifest = crate::plugin::PluginPackage::manifest_of(compose, &langs).unwrap();
        assert!(
            manifest.webui.as_ref().is_some_and(|webui| webui.guests),
            "compose 要声明 webui.guests"
        );
        let id = manifest.id.clone();
        self.manager.register_for_test(
            manifest.clone(),
            crate::plugin::PluginStatus::Running,
            Some(self.backend.to_string()),
        );
        let instance = crate::plugin::names::Instance::default();
        self.docker
            .add_unlabeled_network(&instance.network(&manifest.id.slug()));
        self.docker.add_endpoint(
            &instance.network(&manifest.id.slug()),
            "wonton-core",
            serde_json::json!({"IPAddress": "10.90.0.3"}),
        );
        if let Some(http) = manifest
            .clients
            .as_ref()
            .and_then(|clients| clients.http.clone())
        {
            let container = instance.container(&manifest.id.slug(), &http.service);
            self.docker.add_image_container(&container, "example/app:1");
            self.manager
                .clients_route_for_test(&manifest.id, http, Some(self.backend.to_string()));
        }
        if manifest
            .webui
            .as_ref()
            .is_some_and(|webui| webui.proxy.is_some())
        {
            let auth = crate::plugin::proxy_auth::ProxyAuth::resolve(
                &manifest,
                &Default::default(),
                &langs,
            )
            .unwrap()
            .expect("sso: proxy 解析出凭据");
            self.manager.proxy_for_test(&manifest.id, auth);
        }
        self.manager
            .clients_entry_for_test(&manifest.id, label, lan, public);
        let mut record = self.manager.record_for_test(&manifest.id).unwrap();
        record.source = compose.to_owned();
        record.granted_permissions = vec![crate::plugin::Permission::NetworkClients];
        self.manager.store().save(&record).unwrap();
        id
    }

    /// 主人令牌的一次客户端接口调用。
    async fn api(&self, method: Method, path: &str, body: &str) -> Reply {
        let bearer = format!("Bearer {}", crate::gateway::tests::TOKEN);
        self.main_request(
            method,
            self.gateway.public.base_domain.as_str(),
            path,
            &[
                ("authorization", &bearer),
                ("x-wonton-clients", "1"),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await
    }

    /// 建一条链接，返回（id，令牌），令牌从返回的地址末段取。
    async fn guest(&self, id: &PluginId, path: Option<&str>) -> (String, String) {
        let body = serde_json::json!({"label": "聚会", "ttl_seconds": 0, "path": path});
        let reply = self
            .api(
                Method::POST,
                &wonton_protocol::clients_guests_path(id.as_str()),
                &body.to_string(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text());
        let json = reply.json();
        let url = json["url"].as_str().unwrap();
        let token = url.rsplit('/').next().unwrap().to_owned();
        (json["id"].as_str().unwrap().to_owned(), token)
    }

    /// 打开链接，返回响应。
    async fn open(&self, host: &str, entry: &str, token: &str) -> Reply {
        self.get(
            host,
            &format!("/_wonton/guest/{token}"),
            &[("x-wonton-entry", entry)],
        )
        .await
    }

    fn crt_list(&self, kind: &str) -> String {
        std::fs::read_to_string(self.install.join(format!("entry/certs/{kind}.crt-list"))).unwrap()
    }

    fn cookie_header(token: &str) -> String {
        format!("{COOKIE_NAME}={token}")
    }
}

#[tokio::test]
async fn guests_only_for_declared_plugins() {
    let home = Home::start().await;
    let guests = home.register_guests(GUEST_ONLY, "guestapp", true, false);
    home.register(ENTRY_APP, "plainapp", true, false);
    let guest_host = home.lan_host("guestapp");
    let plain_host = home.lan_host("plainapp");

    assert_eq!(
        home.open(&guest_host, "lan", "nothing").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        home.open(&plain_host, "lan", "nothing").await.status,
        StatusCode::NOT_FOUND
    );
    // 接口：没声明的一律 404。
    let post = serde_json::json!({"label": "聚会", "ttl_seconds": 0});
    for id in ["io.wonton.test.entry", "io.wonton.test.missing"] {
        let reply = home
            .api(
                Method::POST,
                &wonton_protocol::clients_guests_path(id),
                &post.to_string(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{id}");
    }
    let reply = home
        .api(
            Method::POST,
            &wonton_protocol::clients_guests_path(guests.as_str()),
            &post.to_string(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
}

#[tokio::test]
async fn entry_closed_without_entry() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_ONLY, "guestapp", false, false);
    let post = serde_json::json!({"label": "聚会", "ttl_seconds": 0});
    let closed = home
        .api(
            Method::POST,
            &wonton_protocol::clients_guests_path(id.as_str()),
            &post.to_string(),
        )
        .await;
    assert_eq!(closed.status, StatusCode::CONFLICT);
    assert_eq!(closed.error_kind(), Some("entry_closed"));

    // 先确认 guest-only 真能走 apply（不被当成“没这个能力”）：总开关开着时打开，公网证书列表里有这个标签。
    let opened = home.switch(id.as_str(), "public", true).await;
    assert_eq!(opened.status, StatusCode::OK, "{}", opened.text());
    assert_eq!(opened.json()["blocked"], serde_json::Value::Null);
    assert_eq!(opened.json()["guests"], true);
    assert_eq!(opened.json()["auth"], "app");
    assert_eq!(opened.json()["entries"].as_array().unwrap().len(), 2);
    assert!(
        home.crt_list("public").contains("guestapp."),
        "{}",
        home.crt_list("public")
    );
    // 再关掉总开关：开关留着，但如实报 public_access_off，链接建不了。
    home.gateway
        .public_access
        .store(false, std::sync::atomic::Ordering::Relaxed);
    let closed = home.switch(id.as_str(), "public", true).await;
    assert_eq!(closed.status, StatusCode::OK, "{}", closed.text());
    assert_eq!(closed.json()["blocked"], "public_access_off");
    let off = home
        .api(
            Method::POST,
            &wonton_protocol::clients_guests_path(id.as_str()),
            &post.to_string(),
        )
        .await;
    assert_eq!(off.status, StatusCode::CONFLICT);
    assert_eq!(off.error_kind(), Some("entry_closed"));
}

#[tokio::test]
async fn ttl_default_and_max_checked() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_ONLY, "guestapp", true, false);
    let body = serde_json::json!({"label": "长", "ttl_seconds": GUEST_TTL_MAX + 1});
    let too_long = home
        .api(
            Method::POST,
            &wonton_protocol::clients_guests_path(id.as_str()),
            &body.to_string(),
        )
        .await;
    assert_eq!(too_long.status, StatusCode::BAD_REQUEST);
    assert_eq!(too_long.error_kind(), Some("invalid_input"));

    let (_, token) = home.guest(&id, None).await;
    let created = home
        .api(
            Method::GET,
            &wonton_protocol::clients_guest_path(id.as_str(), "g-1"),
            "",
        )
        .await;
    assert_eq!(created.status, StatusCode::OK);
    let expires = created.json()["expires_at_ms"].as_u64().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let span = expires - now;
    assert!(
        span > 4 * 3600 * 1000 - 10_000 && span <= 4 * 3600 * 1000,
        "{span}"
    );
    assert_eq!(
        created.json()["url"].as_str().unwrap(),
        format!(
            "https://guestapp.lan.home.example.test:{}/_wonton/guest/{token}",
            crate::clients::external_url::lan_port()
        )
    );
}

#[tokio::test]
async fn max_twenty_active_links() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_ONLY, "guestapp", true, false);
    for _ in 0..GUEST_LINKS_MAX {
        let _ = home.guest(&id, None).await;
    }
    let body = serde_json::json!({"label": "多出来的", "ttl_seconds": 0});
    let extra = home
        .api(
            Method::POST,
            &wonton_protocol::clients_guests_path(id.as_str()),
            &body.to_string(),
        )
        .await;
    assert_eq!(extra.status, StatusCode::CONFLICT);
    assert_eq!(extra.error_kind(), Some("conflict"));
    let list = home
        .api(
            Method::GET,
            &wonton_protocol::clients_guests_path(id.as_str()),
            "",
        )
        .await;
    assert_eq!(list.json().as_array().unwrap().len(), GUEST_LINKS_MAX);
    assert!(list.json()[0]["url"].is_null());
}

#[tokio::test]
async fn landing_path_must_be_same_origin() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_ONLY, "guestapp", true, false);
    for path in ["room", "//elsewhere.test/room"] {
        let body = serde_json::json!({"label": "聚会", "ttl_seconds": 0, "path": path});
        let reply = home
            .api(
                Method::POST,
                &wonton_protocol::clients_guests_path(id.as_str()),
                &body.to_string(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{path}");
    }
}

#[tokio::test]
async fn exchange_sets_host_cookie_and_no_referrer() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_ONLY, "guestapp", true, false);
    let (guest, token) = home.guest(&id, Some("/room/abc")).await;
    let host = home.lan_host("guestapp");
    let reply = home.open(&host, "lan", &token).await;
    assert_eq!(reply.status, StatusCode::FOUND);
    assert_eq!(reply.location(), "/room/abc");
    assert_eq!(reply.headers[header::REFERER], "no-referrer");
    assert_eq!(reply.headers[header::CACHE_CONTROL], "no-store");
    let cookie = reply
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .find(|value| value.starts_with(&format!("{COOKIE_NAME}=")))
        .expect("响应里要有访客 Cookie");
    assert!(cookie.contains("Path=/;"), "{cookie}");
    assert!(cookie.contains("Secure"), "{cookie}");
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Lax"), "{cookie}");
    assert!(cookie.contains("Max-Age="), "{cookie}");
    assert!(!cookie.contains("Domain="), "{cookie}");
    assert!(cookie.contains(&token), "{cookie}");

    // 不写落地路径时回首页。
    let (_, plain) = home.guest(&id, None).await;
    assert_eq!(home.open(&host, "lan", &plain).await.location(), "/");
    // 非 ASCII 的落地路径要编码进 `Location`，不能静默落回首页。
    let (_, chinese) = home.guest(&id, Some("/房间")).await;
    assert_eq!(
        home.open(&host, "lan", &chinese).await.location(),
        "/%E6%88%BF%E9%97%B4"
    );

    // 已经收回的链接 409。
    let revoke = home
        .api(
            Method::DELETE,
            &wonton_protocol::clients_guest_path(id.as_str(), &guest),
            "",
        )
        .await;
    assert_eq!(revoke.status, StatusCode::NO_CONTENT);
    let gone = home
        .api(
            Method::GET,
            &wonton_protocol::clients_guest_path(id.as_str(), &guest),
            "",
        )
        .await;
    assert_eq!(gone.status, StatusCode::CONFLICT);
    assert_eq!(gone.error_kind(), Some("entry_closed"));
}

#[tokio::test]
async fn wrong_token_counted_and_expiry_drops_next_request() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_ONLY, "guestapp", true, false);
    let host = home.lan_host("guestapp");
    // 错的令牌带计数头，入口容器据此封来源。
    let wrong = home.open(&host, "lan", "not-a-token").await;
    assert_eq!(wrong.status, StatusCode::FORBIDDEN);
    assert_eq!(wrong.headers["x-wonton-auth-failure"], "1");

    // 过期的链接下一个请求就失效。
    let body = serde_json::json!({"label": "一分钟", "ttl_seconds": 1});
    let reply = home
        .api(
            Method::POST,
            &wonton_protocol::clients_guests_path(id.as_str()),
            &body.to_string(),
        )
        .await;
    let url = reply.json()["url"].as_str().unwrap().to_owned();
    let token = url.rsplit('/').next().unwrap();
    assert_eq!(
        home.open(&host, "lan", token).await.status,
        StatusCode::FOUND
    );
    let started = reply.json()["id"].as_str().unwrap().to_owned();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let after = home
        .get(
            &host,
            "/room",
            &[
                ("x-wonton-entry", "lan"),
                ("cookie", &Home::cookie_header(token)),
            ],
        )
        .await;
    assert_eq!(after.status, StatusCode::FORBIDDEN);
    let gone = home
        .api(
            Method::GET,
            &wonton_protocol::clients_guest_path(id.as_str(), &started),
            "",
        )
        .await;
    assert_eq!(gone.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn token_not_forwarded_and_identity_cookies_stripped() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_AND_HTTP, "guestapp", true, false);
    let (_, token) = home.guest(&id, None).await;
    let host = home.lan_host("guestapp");
    let cookie = format!(
        "{}; wonton_session=core; wonton_app=app; app_session=keep",
        Home::cookie_header(&token)
    );
    let reply = home
        .get(
            &host,
            "/room",
            &[
                ("x-wonton-entry", "lan"),
                ("cookie", &cookie),
                ("x-remote-user", "spoofed"),
                ("x-wonton-plugin", "spoofed"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let hit = home.upstream_hits().last().unwrap().clone();
    assert_eq!(hit.uri, "/room");
    assert_eq!(hit.cookie(), "app_session=keep");
    assert!(!hit.has("x-remote-user"));
    assert!(!hit.has("x-wonton-plugin"));
    assert!(!hit.has("authorization"));
}

#[tokio::test]
async fn guest_only_plugin_rejects_without_cookie_and_outside_scope() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_ONLY, "guestapp", true, false);
    let (_, token) = home.guest(&id, Some("/room")).await;
    let host = home.lan_host("guestapp");
    let entries = [("x-wonton-entry", "lan")];

    // 没 Cookie：到不了应用。
    let none = home.get(&host, "/room", &entries).await;
    assert_eq!(none.status, StatusCode::FORBIDDEN);
    assert!(home.upstream_hits().is_empty());

    // Cookie 在，但请求在声明范围外：同样到不了。
    let cookie = Home::cookie_header(&token);
    let outside = home
        .get(
            &host,
            "/elsewhere",
            &[("x-wonton-entry", "lan"), ("cookie", &cookie)],
        )
        .await;
    assert_eq!(outside.status, StatusCode::FORBIDDEN);
    assert!(home.upstream_hits().is_empty());

    // 范围内放行。
    let inside = home
        .get(
            &host,
            "/room/abc",
            &[("x-wonton-entry", "lan"), ("cookie", &cookie)],
        )
        .await;
    assert_eq!(inside.status, StatusCode::OK);
}

#[tokio::test]
async fn revoked_link_is_a_403_invalid_page() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_ONLY, "guestapp", true, false);
    let (guest, token) = home.guest(&id, None).await;
    let host = home.lan_host("guestapp");
    let revoke = home
        .api(
            Method::DELETE,
            &wonton_protocol::clients_guest_path(id.as_str(), &guest),
            "",
        )
        .await;
    assert_eq!(revoke.status, StatusCode::NO_CONTENT);
    let after = home
        .get(
            &host,
            "/room",
            &[
                ("x-wonton-entry", "lan"),
                ("cookie", &Home::cookie_header(&token)),
            ],
        )
        .await;
    assert_eq!(after.status, StatusCode::FORBIDDEN);
    assert_eq!(after.error_kind(), Some("entry_closed"));
}

#[tokio::test]
async fn guest_cookie_beats_entry_auth_only_inside_scope() {
    let home = Home::start().await;
    let id = home.register_guests(GUEST_AND_HTTP, "guesthttp", true, false);
    let (_guest, token) = home.guest(&id, Some("/room")).await;
    let host = home.lan_host("guesthttp");
    let cookie = Home::cookie_header(&token);

    let inside = home
        .get(
            &host,
            "/room",
            &[("x-wonton-entry", "lan"), ("cookie", &cookie)],
        )
        .await;
    assert_eq!(inside.status, StatusCode::OK);

    // 范围外退回入口自己的密码：没带 Basic 就是 401。
    let outside = home
        .get(
            &host,
            "/elsewhere",
            &[("x-wonton-entry", "lan"), ("cookie", &cookie)],
        )
        .await;
    assert_eq!(outside.status, StatusCode::UNAUTHORIZED);

    // 收回后已升级的连接立即断，下一个请求也不放行。整站链接才管得到 `/ws`。
    let (whole, whole_token) = home.guest(&id, None).await;
    let mut stream = connect_upgraded(&home, &host, &Home::cookie_header(&whole_token)).await;
    let revoke = home
        .api(
            Method::DELETE,
            &wonton_protocol::clients_guest_path(id.as_str(), &whole),
            "",
        )
        .await;
    assert_eq!(revoke.status, StatusCode::NO_CONTENT);
    let mut closed = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut closed)).await;
    assert!(read.is_ok(), "收回后连接应立即断开");
}

/// 用访客 Cookie 打开一个协议升级的连接，并等回显确认已经升级。
async fn connect_upgraded(home: &Home, host: &str, cookie: &str) -> tokio::net::TcpStream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (listener, addr) = crate::gateway::fakes::bind().await;
    crate::gateway::fakes::serve(listener, home.app.clone()).await;
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: echo\r\n\
         x-wonton-entry: lan\r\nCookie: {cookie}; app_session=keep\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    stream.write_all(b"ping").await.unwrap();
    let mut frame = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut frame))
        .await
        .expect("等到回显")
        .unwrap();
    assert_eq!(&frame, b"ping");
    // `app_session` 还留着，说明访客 Cookie 被删掉、别的 Cookie 没事。
    let hit = home.upstream_hits().last().unwrap().clone();
    assert_eq!(hit.cookie(), "app_session=keep");
    stream
}
