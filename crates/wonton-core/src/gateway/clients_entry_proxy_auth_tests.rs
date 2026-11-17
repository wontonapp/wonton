//! 网关代认证（auth: gateway）：basic 与 bearer 都认、Authorization 不转发、身份按清单模板代加；
//! 缺凭据的质询不计验证失败，错凭据计一次；吊销下一个请求生效，并断开已升级的连接。

use super::*;

const GATEWAY_APP: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: io.wonton.test.gateway
  version: 1.0.0
  kind: app
  title: { zh: 网关示例, en: Gateway example }
  description:
    zh: 网关验证的行为测试用
    en: For gateway auth behaviour tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        gateway:
          headers:
            X-Remote-User: "{user}"
"#;

#[tokio::test]
async fn basic_and_bearer_accepted_authorization_stripped() {
    let home = Home::start().await;
    home.register(GATEWAY_APP, "gwapp", false, true);
    let store = home
        .gateway
        .plugins
        .client_passwords
        .as_ref()
        .expect("密码库随管理器打开");
    let created = store
        .create("io.wonton.test.gateway", "phone", None)
        .unwrap();
    let host = home.host("gwapp");
    use base64::Engine as _;
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(format!("owner:{}", created.value));
    let basic = format!("Basic {encoded}");
    let bearer = format!("Bearer {}", created.value);
    for authorization in [&basic, &bearer] {
        let reply = home
            .get(
                &host,
                "/",
                &[
                    ("x-wonton-entry", "public"),
                    ("authorization", authorization),
                ],
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{authorization}");
    }
    // 两种方案都过；Authorization 不转发，身份按清单模板代加。
    let hits = home.upstream_hits();
    assert_eq!(hits.len(), 2);
    for hit in &hits {
        assert!(!hit.has("authorization"), "{hit:?}");
        assert_eq!(hit.value("x-remote-user"), Some("owner"));
        assert_eq!(hit.value("x-forwarded-proto"), Some("https"));
    }
}

#[tokio::test]
async fn missing_auth_basic_challenge_not_counted() {
    let home = Home::start().await;
    home.register(GATEWAY_APP, "gwapp", false, true);
    let host = home.host("gwapp");
    let reply = home.get(&host, "/", &entry_public()).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.error_kind(), Some("unauthenticated"));
    assert_eq!(
        reply
            .headers
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Basic realm=\"wonton\"")
    );
    // 没带凭据的质询是 DAV、Git 客户端的第一步，不算验证失败。
    assert!(reply.headers.get("x-wonton-auth-failure").is_none());
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn wrong_auth_counted() {
    let home = Home::start().await;
    home.register(GATEWAY_APP, "gwapp", false, true);
    let store = home.gateway.plugins.client_passwords.as_ref().unwrap();
    let created = store
        .create("io.wonton.test.gateway", "phone", None)
        .unwrap();
    let host = home.host("gwapp");
    use base64::Engine as _;
    let wrong_value =
        base64::engine::general_purpose::STANDARD.encode(format!("owner:not-{}", created.value));
    let wrong_user = base64::engine::general_purpose::STANDARD.encode("mallory:x");
    for authorization in [
        format!("Basic {wrong_value}"),
        format!("Basic {wrong_user}"),
        "Bearer garbage".to_owned(),
        "Digest x".to_owned(),
    ] {
        let reply = home
            .get(
                &host,
                "/",
                &[
                    ("x-wonton-entry", "public"),
                    ("authorization", &authorization),
                ],
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{authorization}");
        assert_eq!(
            reply
                .headers
                .get("x-wonton-auth-failure")
                .map(|v| v.to_str().unwrap()),
            Some("1"),
            "{authorization}"
        );
    }
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn revoke_effective_next_request_and_drops_websocket() {
    let home = Home::start().await;
    home.register(GATEWAY_APP, "gwapp", false, true);
    let store = home.gateway.plugins.client_passwords.as_ref().unwrap();
    let created = store
        .create("io.wonton.test.gateway", "phone", None)
        .unwrap();
    let host = home.host("gwapp");
    let bearer = format!("Bearer {}", created.value);
    let headers = [
        ("x-wonton-entry", "public"),
        ("authorization", bearer.as_str()),
    ];
    let ok = home.get(&host, "/", &headers).await;
    assert_eq!(ok.status, StatusCode::OK);

    // 已升级的连接：吊销后立即断。
    let (listener, addr) = bind().await;
    serve(listener, home.app.clone()).await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: echo\r\nx-wonton-entry: public\r\nauthorization: {bearer}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut byte))
            .await
            .expect("等到升级响应")
            .unwrap();
        head.push(byte[0]);
    }
    assert!(
        head.starts_with(b"HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&head)
    );
    stream.write_all(b"ping").await.unwrap();
    let mut frame = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut frame))
        .await
        .expect("等到回显")
        .unwrap();
    assert_eq!(&frame, b"ping");

    // 吊销：升级中的连接立即断；下一个请求 401，带了凭据按验证失败计数。
    store.revoke("io.wonton.test.gateway", &created.id).unwrap();
    let mut closed = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut closed)).await;
    assert!(read.is_ok(), "吊销后连接应立即断开");
    let denied = home.get(&host, "/", &headers).await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        denied
            .headers
            .get("x-wonton-auth-failure")
            .map(|v| v.to_str().unwrap()),
        Some("1")
    );
}
