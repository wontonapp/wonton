//! 头的剥离与代加：身份类与云吞保留头到不了后端；webui 代理头按规范化名剥离；
//! owner 的 cookie 不当身份。

use super::*;

#[tokio::test]
async fn identity_headers_and_wonton_headers_stripped() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let reply = home
        .get(
            &host,
            "/",
            &[
                ("x-wonton-entry", "public"),
                ("x-wonton-spoofed", "1"),
                ("x-remote-user", "mallory"),
                ("x-real-ip", "203.0.113.7"),
                ("authorization", "Bearer app-own-token"),
                (
                    "cookie",
                    "app_session=keep; wonton_session=core; Wonton_X=1",
                ),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let hit = &home.upstream_hits()[0];
    assert_eq!(hit.method, Method::GET);
    assert!(!hit.has("x-remote-user"), "{hit:?}");
    assert!(!hit.has("x-wonton-spoofed"));
    assert!(!hit.has("x-wonton-entry"));
    assert_eq!(hit.value("authorization"), Some("Bearer app-own-token"));
    assert_eq!(hit.cookie(), "app_session=keep");
    // 入口写的真实来源照转，转发头由核心重写。
    assert_eq!(hit.value("x-real-ip"), Some("203.0.113.7"));
    assert_eq!(hit.value("x-forwarded-for"), Some("203.0.113.7"));
    assert_eq!(hit.value("x-forwarded-host"), Some(host.as_str()));
    assert_eq!(hit.value("x-forwarded-proto"), Some("https"));
}

#[tokio::test]
async fn webui_proxy_headers_stripped_by_normalized_name() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    home.get(
        &host,
        "/",
        &[
            ("x-wonton-entry", "public"),
            ("x_remote_user", "mallory"),
            ("X_REMOTE_USER", "mallory-again"),
        ],
    )
    .await;
    let hit = &home.upstream_hits()[0];
    assert!(!hit.has("x_remote_user"), "{hit:?}");
    assert!(!hit.has("x-remote-user"), "{hit:?}");
}

#[tokio::test]
async fn owner_cookie_never_becomes_identity() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let reply = home
        .get(
            &host,
            "/",
            &[
                ("x-wonton-entry", "public"),
                ("cookie", "wonton_session=owner-core-session"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let hit = &home.upstream_hits()[0];
    assert_eq!(hit.cookie(), "");
    // 这个入口不代登录：没有网关代加的身份头。
    assert!(!hit.has("x-remote-user"));
    assert!(!hit.has("x-wonton-owner"));
}
