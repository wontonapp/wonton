//! 链接预览接口（ADR-0065）的网关行为：谁能调、失败按类别给状态码，图片过期后 404。取预览本身的行为见
//! `crate::link_preview` 的测试。

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::get;
use hyper::{Method, StatusCode, header};
use iroh::SecretKey;
use wonton_protocol::{
    LINK_PREVIEW_PATH, LinkPreview, LinkPreviewFailure, link_preview_image_path,
};

use crate::config::PublicUrls;
use crate::gateway::family_tests::{PORT, send_as};
use crate::gateway::tests::{CORE, Harness, Reply, TOKEN, owner};
use crate::gateway::{ERROR_HEADER, Gateway, router};
use crate::link_preview::test_support::{Handles, og_page, png, world_with};
use crate::link_preview::{Limits, LinkPreviews};
use crate::members::MemberRecord;
use crate::owner::OwnerStore;
use crate::plugin::egress::address::ProxyAddress;
use crate::plugin::languages::Languages;

impl Gateway {
    #[must_use]
    fn with_link_previews(mut self, previews: LinkPreviews) -> Self {
        self.link_previews = Arc::new(previews);
        self
    }
}

fn site() -> Router {
    Router::new()
        .route(
            "/post",
            get(|| async { axum::response::Html(og_page("Hello", Some("/img.png"))) }),
        )
        .route(
            "/img.png",
            get(|| async { ([(header::CONTENT_TYPE, "image/png")], png(3, 2)).into_response() }),
        )
        .route(
            "/other",
            get(|| async { axum::response::Html(og_page("Other", None)) }),
        )
        .route(
            "/third",
            get(|| async { axum::response::Html(og_page("Third", None)) }),
        )
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                axum::response::Html(og_page("Slow", None))
            }),
        )
}

/// 网关接上测试用的链接预览，其余和 `Harness` 的网关相同。
fn app_with(h: &Harness, previews: LinkPreviews) -> Router {
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: PORT,
    };
    let gateway = Gateway::new(
        public,
        OwnerStore::for_test(h.data_dir(), &owner()),
        TOKEN.into(),
        h.members.clone(),
        h.manager.clone(),
        None,
        h.http.clone(),
        h.tunnel.clone(),
    )
    .with_link_previews(previews);
    router(Arc::new(gateway))
}

async fn setup(limits: Limits) -> (Harness, Router, Handles) {
    setup_with_dns(limits, &[]).await
}

/// 同 [`setup`]，另给几个域名指向的地点。
async fn setup_with_dns(limits: Limits, dns: &[(&str, &[&str])]) -> (Harness, Router, Handles) {
    let h = Harness::start(None).await;
    let (previews, world) = world_with(site(), dns, limits, None).await;
    let app = app_with(&h, previews);
    (h, app, world)
}

fn preview_path(url: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("url", url)
        .finish();
    format!("{LINK_PREVIEW_PATH}?{query}")
}

fn kind(reply: &Reply) -> LinkPreviewFailure {
    assert_eq!(
        reply.error_kind(),
        reply.json()["error"]["kind"].as_str(),
        "响应头和响应体的类别要一致"
    );
    serde_json::from_value(reply.json()["error"]["kind"].clone()).unwrap()
}

fn invite(h: &Harness, username: &str) {
    h.members
        .invite(MemberRecord {
            invited_by: None,
            username: username.to_owned(),
            user_id: format!("@{username}:test"),
            display_name: username.to_owned(),
            email: None,
            status: wonton_protocol::MemberStatus::Invited,
            role_id: "friend".to_owned(),
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
}

#[tokio::test]
async fn owner_and_family_devices_get_previews_but_bots_and_plugin_pages_do_not() {
    let (h, app, world) = setup(Limits::default()).await;
    let owners = SecretKey::generate().public();
    let hers = SecretKey::generate().public();
    h.tunnel.devices.add(owners, None).unwrap();
    invite(&h, "mia");
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    let path = preview_path("http://site.test/post");
    for device in [owners, hers] {
        let reply = send_as(&app, Some(device), Method::GET, CORE, &path, &[], "").await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        let preview: LinkPreview = serde_json::from_str(reply.text()).unwrap();
        assert_eq!(preview.title.as_deref(), Some("Hello"));
    }
    let bearer = format!("Bearer {TOKEN}");
    let reply = send_as(
        &app,
        None,
        Method::GET,
        CORE,
        &path,
        &[("authorization", &bearer)],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());

    // Bot 拿着自己的 Matrix 令牌、插件容器什么都不带：都不认。
    let bot = [("authorization", "Bearer syt_Ym90_matrix_token")];
    for headers in [&bot[..], &[]] {
        let reply = send_as(&app, None, Method::GET, CORE, &path, headers, "").await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
    }
    // App 里打开的插件网页借设备的隧道身份发请求：不是核心自己的页面，拒绝。
    let plugin_origin = format!("http://app-whoami.{CORE}:{PORT}");
    let from_plugin = [
        ("sec-fetch-site", "same-site"),
        ("origin", plugin_origin.as_str()),
    ];
    let image = link_preview_image_path(&"0".repeat(64));
    for path in [path.as_str(), image.as_str()] {
        let reply = send_as(
            &app,
            Some(owners),
            Method::GET,
            CORE,
            path,
            &from_plugin,
            "",
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
        let reply = send_as(&app, None, Method::GET, CORE, path, &bot, "").await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
    }
    assert_eq!(world.hits_of("/post"), 1, "拒绝的请求不该去取");
}

#[tokio::test]
async fn failures_carry_their_category_and_a_matching_status() {
    let limits = Limits {
        per_minute: 1,
        ..Limits::default()
    };
    let (h, app, _world) = setup(limits).await;
    let phone = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    for (path, status, expected) in [
        (
            LINK_PREVIEW_PATH.to_owned(),
            StatusCode::BAD_REQUEST,
            LinkPreviewFailure::InvalidUrl,
        ),
        (
            preview_path("http://site.test:9000/"),
            StatusCode::BAD_REQUEST,
            LinkPreviewFailure::InvalidUrl,
        ),
        (
            preview_path("http://127.0.0.1/"),
            StatusCode::UNPROCESSABLE_ENTITY,
            LinkPreviewFailure::BlockedAddress,
        ),
        (
            preview_path("http://site.test/missing"),
            StatusCode::TOO_MANY_REQUESTS,
            LinkPreviewFailure::TooManyRequests,
        ),
    ] {
        let reply = send_as(&app, Some(phone), Method::GET, CORE, &path, &[], "").await;
        assert_eq!(reply.status, status, "{path}：{}", reply.text());
        assert_eq!(kind(&reply), expected, "{path}");
    }
}

#[tokio::test]
async fn the_image_is_served_until_the_preview_expires() {
    let (h, app, world) = setup(Limits::default()).await;
    let phone = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    let path = preview_path("http://site.test/post");
    let reply = send_as(&app, Some(phone), Method::GET, CORE, &path, &[], "").await;
    let preview: LinkPreview = serde_json::from_str(reply.text()).unwrap();
    let image = link_preview_image_path(&preview.image.unwrap().id);

    let reply = send_as(&app, Some(phone), Method::GET, CORE, &image, &[], "").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(reply.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

    world.advance(Duration::from_secs(24 * 3600));
    let reply = send_as(&app, Some(phone), Method::GET, CORE, &image, &[], "").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.headers.contains_key(ERROR_HEADER));
}

#[tokio::test]
async fn a_dns_failure_is_reported_as_its_own_category() {
    let (h, app, world) = setup(Limits::default()).await;
    let phone = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    // 代理地址上没人，经代理的 DoH 连不上。
    world.proxy.set(Some(
        &ProxyAddress::parse("http://127.0.0.1:1", &Languages::default()).unwrap(),
    ));

    let reply = send_as(
        &app,
        Some(phone),
        Method::GET,
        CORE,
        &preview_path("http://site.test/"),
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY, "{}", reply.text());
    assert_eq!(kind(&reply), LinkPreviewFailure::DnsFailed);
}

#[tokio::test]
async fn private_addresses_are_blocked_at_the_gateway_without_connecting() {
    let dns: &[(&str, &[&str])] = &[
        ("home.test", &["192.168.1.10"]),
        ("loop.test", &["127.0.0.1"]),
        ("link.test", &["169.254.169.254"]),
        ("cgnat.test", &["100.64.0.1"]),
        ("ula.test", &["fd00::1"]),
    ];
    let (h, app, world) = setup_with_dns(Limits::default(), dns).await;
    let phone = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    for url in [
        "http://10.1.2.3/",
        "http://172.16.0.1/",
        "http://192.168.1.1/",
        "http://127.0.0.1/",
        "http://[::1]/",
        "http://169.254.169.254/latest/meta-data/",
        "http://100.64.0.1/",
        "http://[fd00::1]/",
        "http://home.test/",
        "http://loop.test/",
        "http://link.test/",
        "http://cgnat.test/",
        "http://ula.test/",
    ] {
        let path = preview_path(url);
        let reply = send_as(&app, Some(phone), Method::GET, CORE, &path, &[], "").await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{url}：{}",
            reply.text()
        );
        assert_eq!(kind(&reply), LinkPreviewFailure::BlockedAddress, "{url}");
    }
    assert!(
        world.dialed.lock().unwrap().is_empty(),
        "内网地址一个都不该去连"
    );
    assert_eq!(world.hit_count(), 0, "内网地址不该打到假站点");
}

#[tokio::test]
async fn cache_hits_do_not_spend_the_rate_limit() {
    let limits = Limits {
        per_minute: 2,
        ..Limits::default()
    };
    let (h, app, world) = setup(limits).await;
    let phone = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    // 同一网址取三次：只有第一次没命中缓存，只算一次。
    for _ in 0..3 {
        let reply = send_as(
            &app,
            Some(phone),
            Method::GET,
            CORE,
            &preview_path("http://site.test/post"),
            &[],
            "",
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    }
    // 第二个不同的网址用掉第二次额度。
    let reply = send_as(
        &app,
        Some(phone),
        Method::GET,
        CORE,
        &preview_path("http://site.test/other"),
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    // 第三个不同的网址是第 3 次，超了。
    let reply = send_as(
        &app,
        Some(phone),
        Method::GET,
        CORE,
        &preview_path("http://site.test/third"),
        &[],
        "",
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        reply.text()
    );
    assert_eq!(kind(&reply), LinkPreviewFailure::TooManyRequests);
    assert_eq!(world.hits_of("/post"), 1, "命中缓存还去取了一次");
    assert_eq!(world.hits_of("/third"), 0, "被限速的不该发出去");
}

#[tokio::test]
async fn each_device_and_the_owner_token_spend_their_own_quota() {
    let limits = Limits {
        per_minute: 1,
        ..Limits::default()
    };
    let (h, app, _world) = setup(limits).await;
    let phone = SecretKey::generate().public();
    let tablet = SecretKey::generate().public();
    let friend_phone = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    h.tunnel.devices.add(tablet, None).unwrap();
    invite(&h, "mia");
    h.tunnel
        .devices
        .add_if(friend_phone, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    let urls = [
        "http://site.test/post",
        "http://site.test/other",
        "http://site.test/third",
        "http://www.site.test/post",
    ];
    let get = |device, url: &str| {
        let path = preview_path(url);
        let app = app.clone();
        async move { send_as(&app, Some(device), Method::GET, CORE, &path, &[], "").await }
    };
    // 主人的第一台设备用完自己的额度。
    assert_eq!(get(phone, urls[0]).await.status, StatusCode::OK);
    assert_eq!(
        get(phone, urls[1]).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    // 主人的第二台设备和家人各自的额度不受影响。
    assert_eq!(get(tablet, urls[1]).await.status, StatusCode::OK);
    assert_eq!(get(friend_phone, urls[2]).await.status, StatusCode::OK);
    // 主人令牌没有设备身份，用它自己的键，也不占设备的额度。
    let bearer = format!("Bearer {TOKEN}");
    let reply = send_as(
        &app,
        None,
        Method::GET,
        CORE,
        &preview_path(urls[3]),
        &[("authorization", &bearer)],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
}

#[tokio::test]
async fn the_same_url_is_only_fetched_once_at_a_time() {
    let limits = Limits {
        per_minute: 1,
        ..Limits::default()
    };
    let (h, app, world) = setup(limits).await;
    let phone = SecretKey::generate().public();
    let tablet = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    h.tunnel.devices.add(tablet, None).unwrap();
    let path = preview_path("http://site.test/slow");
    let (first, second) = tokio::join!(
        send_as(&app, Some(phone), Method::GET, CORE, &path, &[], ""),
        send_as(&app, Some(tablet), Method::GET, CORE, &path, &[], ""),
    );
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(second.status, StatusCode::OK, "{}", second.text());
    assert_eq!(world.hits_of("/slow"), 1, "同一个网址同时取了两次");
    // 等待的那台没有花自己的额度：额度只有 1 次，它还能再取另一个网址。
    let reply = send_as(
        &app,
        Some(tablet),
        Method::GET,
        CORE,
        &preview_path("http://site.test/other"),
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(world.hits_of("/other"), 1);
}

/// 领头那台自己的额度用完了，等着的这台还没花过，应该拿它自己的额度再走一次。
#[tokio::test]
async fn a_waiter_out_of_its_own_quota_still_gets_the_card() {
    let limits = Limits {
        per_minute: 1,
        ..Limits::default()
    };
    let (h, app, world) = setup(limits).await;
    let phone = SecretKey::generate().public();
    let tablet = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    h.tunnel.devices.add(tablet, None).unwrap();
    // 手机先把这一分钟的额度用光。
    let spent = send_as(
        &app,
        Some(phone),
        Method::GET,
        CORE,
        &preview_path("http://site.test/other"),
        &[],
        "",
    )
    .await;
    assert_eq!(spent.status, StatusCode::OK, "{}", spent.text());
    let path = preview_path("http://site.test/slow");
    let (first, second) = tokio::join!(
        send_as(&app, Some(phone), Method::GET, CORE, &path, &[], ""),
        send_as(&app, Some(tablet), Method::GET, CORE, &path, &[], ""),
    );
    assert_eq!(
        first.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        first.text()
    );
    assert_eq!(
        second.status,
        StatusCode::OK,
        "平板还有额度，不该替手机吃那个 429：{}",
        second.text()
    );
    assert_eq!(world.hits_of("/slow"), 1);
}

/// 被打断的是我们自己的请求，文案不该说成对方网站出错。
#[test]
fn an_interrupted_preview_is_not_blamed_on_the_site() {
    let (status, kind, message) = super::failure_message(LinkPreviewFailure::Other);
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(kind, "other");
    assert_eq!(message, "link-preview-interrupted");
}
