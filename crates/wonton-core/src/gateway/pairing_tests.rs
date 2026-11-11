//! 扫码配对经核心的真实隧道入口：局域网状态页给出首次配对码，手机经 `wonton/pair/0` 出示后成为主人的设备。

use axum::body::Body;
use http_body_util::BodyExt;
use hyper::header;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use tower::ServiceExt;
use wonton_tunnel::pair::{PairResponse, PairingCode, RejectReason, pair};

use super::fakes::loopback_endpoint;
use super::tests::{CORE, TOKEN};
use super::tunnel_tests::{Home, Phone};

/// 局域网里的浏览器用这个地址打开状态页。
const LAN_HOST: &str = "192.168.1.20:18480";

async fn lan_page(home: &Home, host: &str) -> (StatusCode, String) {
    let request = hyper::Request::builder()
        .uri("/")
        .header(header::HOST, host)
        .body(Body::empty())
        .unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// 状态页上“没法扫码”里写的配对码。
fn code_on(page: &str) -> Option<PairingCode> {
    let start = page.find("<code class=\"pairing-code\">")? + "<code class=\"pairing-code\">".len();
    let end = start + page[start..].find("</code>")?;
    let text = page[start..end].replace("&amp;", "&");
    Some(PairingCode::parse(&text).expect("状态页上的码应能解析"))
}

/// 测试里手机只能走回环地址：把码里的直连地址换成服务端实际绑定的地址，密钥不变。
fn reachable(mut code: PairingCode, home: &Home) -> PairingCode {
    code.direct_addrs = home.tunnel.endpoint.bound_sockets();
    code
}

fn reject_reason(response: PairResponse) -> Option<RejectReason> {
    match response {
        PairResponse::Paired => None,
        PairResponse::Rejected { reason, .. } => Some(reason),
    }
}

#[tokio::test]
async fn the_first_code_on_the_lan_page_makes_a_phone_the_owner_once() {
    let home = Home::start(None).await;
    let (status, page) = lan_page(&home, LAN_HOST).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("<svg"), "没有设备时状态页有二维码");
    let code = code_on(&page).expect("状态页上有配对码");
    assert_eq!(code.server_id, home.tunnel.endpoint.id());
    assert_eq!(
        code.direct_addrs,
        vec![
            format!("192.168.1.20:{}", home.tunnel.port)
                .parse()
                .unwrap()
        ],
        "直连地址是浏览器打开状态页用的 IP 加隧道端口"
    );

    let key = SecretKey::generate();
    let endpoint = loopback_endpoint(key.clone()).await;
    let code = reachable(code, &home);
    assert_eq!(
        reject_reason(pair(&endpoint, &code, "Alice 的 iPhone").await.unwrap()),
        None
    );
    assert!(home.tunnel.devices.contains(&key.public()));

    // 配上后这台手机经隧道就是主人。
    let phone = Phone::start(key, &home).await;
    let (status, _, text) = phone.get(CORE, "/").await;
    assert_eq!(status, StatusCode::OK, "{text}");

    // 状态页不再出码；同一个码别人也用不了。
    let (_, page) = lan_page(&home, LAN_HOST).await;
    assert!(code_on(&page).is_none() && !page.contains("<svg"), "{page}");
    assert!(page.contains("已配对 1 台设备"), "{page}");
    let other = loopback_endpoint(SecretKey::generate()).await;
    assert_eq!(
        reject_reason(pair(&other, &code, "别人的手机").await.unwrap()),
        Some(RejectReason::Used)
    );
}

#[tokio::test]
async fn the_lan_page_only_answers_ip_hosts() {
    let home = Home::start(None).await;
    // DNS 重绑定：别的网站的域名解析到局域网 IP，浏览器带着那个域名来。
    let (status, page) = lan_page(&home, "evil.example:18480").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(code_on(&page).is_none());
    let (status, page) = lan_page(&home, "[fd00::20]:18480").await;
    assert_eq!(status, StatusCode::OK);
    let code = code_on(&page).expect("IPv6 地址也能打开状态页");
    assert_eq!(
        code.direct_addrs[0],
        format!("[fd00::20]:{}", home.tunnel.port).parse().unwrap()
    );
}

/// 服务器上的 `wonton-core pair` 调的管理 API。`body` 里给直连地址；`None` 用一组固定的。
async fn ask_for_code(
    home: &Home,
    path: &str,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    ask_with(
        home,
        path,
        token,
        r#"{"direct_addrs": ["192.168.1.20:18482"]}"#,
    )
    .await
}

/// 同 [`ask_for_code`]，请求体由调用方给。
async fn ask_with(
    home: &Home,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let mut builder = hyper::Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::HOST, format!("{CORE}:18480"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = builder.body(Body::from(body.to_owned())).unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
async fn the_install_terminal_shows_the_lan_pages_first_code_until_a_device_pairs() {
    let home = Home::start(None).await;
    let first = "/_wonton/api/pairing-codes/first";
    let (status, _) = ask_for_code(&home, first, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = ask_for_code(&home, first, Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let on_terminal = PairingCode::parse(body["code"].as_str().unwrap()).unwrap();
    assert_eq!(
        on_terminal.direct_addrs,
        vec!["192.168.1.20:18482".parse().unwrap()]
    );
    let (_, page) = lan_page(&home, LAN_HOST).await;
    let on_page = code_on(&page).unwrap();
    assert_eq!(
        on_terminal.secret.expose(),
        on_page.secret.expose(),
        "终端和状态页上是同一个首次配对码"
    );

    let phone = loopback_endpoint(SecretKey::generate()).await;
    assert_eq!(
        reject_reason(
            pair(&phone, &reachable(on_terminal, &home), "手机")
                .await
                .unwrap()
        ),
        None
    );
    let (status, body) = ask_for_code(&home, first, Some(TOKEN)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["kind"], "paired");
}

/// 请求里没给直连地址时，码里是核心自己算的地址：启动器声明了宣告地址就是宣告地址加隧道端口。
#[tokio::test]
async fn codes_without_addresses_use_the_announced_address() {
    let home = Home::start_lan(&["192.168.0.15"]).await;
    let first = "/_wonton/api/pairing-codes/first";
    let (status, body) = ask_with(&home, first, Some(TOKEN), "{}").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let code = PairingCode::parse(body["code"].as_str().unwrap()).unwrap();
    let addr = format!("192.168.0.15:{}", home.tunnel.port);
    assert_eq!(code.direct_addrs, vec![addr.parse().unwrap()]);

    // 配上第一台后，主人发的码同样用宣告地址。
    let phone = loopback_endpoint(SecretKey::generate()).await;
    assert_eq!(
        reject_reason(pair(&phone, &reachable(code, &home), "手机").await.unwrap()),
        None
    );
    let (status, body) = ask_with(&home, "/_wonton/api/pairing-codes", Some(TOKEN), "{}").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let code = PairingCode::parse(body["code"].as_str().unwrap()).unwrap();
    let addr = format!("192.168.0.15:{}", home.tunnel.port);
    assert_eq!(code.direct_addrs, vec![addr.parse().unwrap()]);
}

/// 没设宣告地址时，没给直连地址的码里是检测到的地址。
#[tokio::test]
async fn codes_without_addresses_use_the_detected_address() {
    let home = Home::start(None).await;
    let (status, body) =
        ask_with(&home, "/_wonton/api/pairing-codes/first", Some(TOKEN), "{}").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let code = PairingCode::parse(body["code"].as_str().unwrap()).unwrap();
    assert_eq!(
        code.direct_addrs,
        vec![format!("192.168.1.2:{}", home.tunnel.port).parse().unwrap()]
    );
}

#[tokio::test]
async fn the_owner_issues_codes_that_add_devices_after_the_first() {
    let home = Home::start(None).await;
    let invite = "/_wonton/api/pairing-codes";
    let (status, _) = ask_for_code(&home, invite, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 先用状态页的首次配对码配上第一台。
    let (_, page) = lan_page(&home, LAN_HOST).await;
    let first = reachable(code_on(&page).unwrap(), &home);
    let phone = loopback_endpoint(SecretKey::generate()).await;
    assert_eq!(
        reject_reason(pair(&phone, &first, "手机").await.unwrap()),
        None
    );

    let (status, body) = ask_for_code(&home, invite, Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["expires_in"], 600);
    let code = PairingCode::parse(body["code"].as_str().unwrap()).unwrap();
    assert_eq!(
        code.direct_addrs,
        vec!["192.168.1.20:18482".parse().unwrap()]
    );
    let tablet_key = SecretKey::generate();
    let tablet = loopback_endpoint(tablet_key.clone()).await;
    assert_eq!(
        reject_reason(
            pair(&tablet, &reachable(code, &home), "平板")
                .await
                .unwrap()
        ),
        None,
        "主人发的码在已有设备时也能加设备"
    );
    assert!(home.tunnel.devices.contains(&tablet_key.public()));
    assert_eq!(home.tunnel.devices.list().len(), 2);
}

#[tokio::test]
async fn a_paired_phone_gets_a_code_for_a_new_device_through_the_tunnel() {
    let home = Home::start(None).await;
    let (_, page) = lan_page(&home, LAN_HOST).await;
    let first = reachable(code_on(&page).unwrap(), &home);
    let phone_key = SecretKey::generate();
    let endpoint = loopback_endpoint(phone_key.clone()).await;
    assert_eq!(
        reject_reason(pair(&endpoint, &first, "手机").await.unwrap()),
        None
    );

    // App 里的“添加设备”：已配对的手机经隧道要码，不带主人令牌。
    let phone = Phone::start(phone_key, &home).await;
    let (status, body) = phone
        .post_json(
            CORE,
            "/_wonton/api/pairing-codes",
            r#"{"direct_addrs": ["192.168.1.20:18482"]}"#,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    let code = PairingCode::parse(body["code"].as_str().unwrap()).unwrap();

    let tablet_key = SecretKey::generate();
    let tablet = loopback_endpoint(tablet_key.clone()).await;
    assert_eq!(
        reject_reason(
            pair(&tablet, &reachable(code, &home), "平板")
                .await
                .unwrap()
        ),
        None
    );
    assert!(home.tunnel.devices.contains(&tablet_key.public()));

    // 局域网里不带令牌的请求照样不行。
    let (status, _) = ask_for_code(&home, "/_wonton/api/pairing-codes", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
