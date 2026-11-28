//! 矩阵入口：只有配对设备的隧道能拿登录令牌、进客户端接口，联邦和管理接口不转发。

use super::*;

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::Value;

use super::super::tests::PLUGIN;

#[tokio::test]
async fn matrix_login_and_client_api_are_only_for_paired_devices() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;
    let matrix = format!("matrix.{CORE}");

    // 已配对设备：两个入口都到得了，核心还没接入 Tuwunel，如实报不可用。
    let (status, headers, _) = phone
        .send(Method::POST, CORE, "/_wonton/matrix/login-token", &[])
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        header(&headers, "x-wonton-error"),
        Some("matrix_unavailable")
    );
    let (status, headers, body) = phone
        .send(Method::GET, &matrix, "/_matrix/client/versions", &[])
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        header(&headers, "x-wonton-error"),
        Some("matrix_unavailable")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["errcode"],
        "M_UNKNOWN"
    );
    // 联邦和管理接口不转发。
    for path in [
        "/_matrix/federation/v1/version",
        "/_synapse/admin/v1/register",
    ] {
        let (status, _, body) = phone.send(Method::GET, &matrix, path, &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        let errcode = serde_json::from_str::<Value>(&body).unwrap()["errcode"].clone();
        assert_eq!(errcode, "M_UNRECOGNIZED", "{path}");
    }
    // 插件网页借着隧道身份跨站要登录令牌：核心域名上的跨站修改拦截挡住它。
    let origin = format!("http://{PLUGIN}:{PORT}");
    let cross_site = [("sec-fetch-site", "same-site"), ("origin", origin.as_str())];
    let (status, headers, _) = phone
        .send(
            Method::POST,
            CORE,
            "/_wonton/matrix/login-token",
            &cross_site,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(header(&headers, "x-wonton-error"), Some("cross_site"));

    // 局域网：没有隧道身份，拿不到登录令牌，也进不了 Matrix（不用 401，免得 SDK 当成令牌失效）。
    let (status, headers, _) = lan(&home, Method::POST, CORE, "/_wonton/matrix/login-token").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(header(&headers, "x-wonton-error"), Some("unauthenticated"));
    let (status, _, body) = lan(&home, Method::GET, &matrix, "/_matrix/client/versions").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["errcode"], "M_FORBIDDEN");
}

#[tokio::test]
async fn paired_devices_get_a_login_token_and_reach_the_client_api() {
    let (fake, config) = crate::matrix::fake::start().await;
    let state = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&config, state.path(), HttpClient::new())
        .await
        .unwrap();
    let home = Home::start_with(None, Some(Arc::new(matrix))).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;
    let matrix_host = format!("matrix.{CORE}");

    let (status, _, body) = phone
        .send(Method::POST, CORE, "/_wonton/matrix/login-token", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token: super::super::matrix::LoginToken = serde_json::from_str(&body).unwrap();
    assert_eq!(token.homeserver_url, format!("http://{matrix_host}:{PORT}"));
    assert_eq!(token.user_id, "@owner:test");
    assert_eq!(token.login_type, "org.matrix.login.jwt");
    assert_eq!(token.expires_in, 120);
    let claims = crate::matrix::issuer::tests::verify(&token.token, &fake.key);
    assert_eq!(claims["sub"], "owner", "令牌只能登录主人");

    // 客户端接口转给 Tuwunel；写法可疑的路径在网关这一层就挡住，到不了管理接口。
    let (status, _, body) = phone
        .send(Method::GET, &matrix_host, "/_matrix/client/versions", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(serde_json::from_str::<Value>(&body).unwrap()["versions"].is_array());
    for path in [
        "/_matrix/client/../../_synapse/admin/v1/register",
        "/_matrix/client/%2e%2e/%2e%2e/_synapse/admin/v1/register",
        "/_matrix/client/.%2e/.%2E/_synapse/admin/v1/register",
    ] {
        let (status, headers, _) = phone.send(Method::GET, &matrix_host, path, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(
            header(&headers, "x-wonton-error"),
            Some("invalid_input"),
            "{path}"
        );
    }
    // 会话 ID 里的 `/` 编成 %2F：按会话取密钥备份的请求原样转给 Tuwunel（这里由假 Tuwunel 回 404 并报出收到的路径）。
    let room_keys = "/_matrix/client/v3/room_keys/keys/!r:test/ab%2Fcd";
    let (status, headers, body) = phone.send(Method::GET, &matrix_host, room_keys, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(header(&headers, "x-wonton-error"), None, "不是网关挡下的");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["path"],
        room_keys
    );
}
