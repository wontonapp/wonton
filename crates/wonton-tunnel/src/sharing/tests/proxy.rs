use super::*;
use crate::ProxyCredential;
use base64::{Engine, engine::general_purpose::STANDARD};
use tokio::net::TcpStream;

async fn proxy_request(addr: std::net::SocketAddr, target: &str, authorization: &str) -> TcpStream {
    let mut tcp = TcpStream::connect(addr).await.unwrap();
    tcp.write_all(format!("GET http://{target}/ HTTP/1.1\r\nHost: {target}\r\nProxy-Authorization: {authorization}\r\n\r\n")
        .as_bytes()).await.unwrap();
    tcp
}

#[tokio::test]
async fn proxy_validates_credentials_and_domain_before_opening_any_forward_stream() {
    let mut setup = Setup::new(None).await;
    let client = setup.client().await;
    let credential = ProxyCredential::generate();
    let proxy = ShareLocalProxy::bind(
        client.clone(),
        (Ipv4Addr::LOCALHOST, 0).into(),
        credential.clone(),
        "home.test".into(),
    )
    .await
    .unwrap();
    let authorization = format!(
        "Basic {}",
        STANDARD.encode(format!(
            "{}:{}",
            credential.username(),
            credential.password()
        ))
    );
    for (host, auth, status) in [
        ("app.home.test", "Basic invalid", "407"),
        ("app.home.test.evil.test", authorization.as_str(), "403"),
    ] {
        let mut tcp = proxy_request(proxy.local_addr(), host, auth).await;
        let mut response = Vec::new();
        timeout(WAIT, tcp.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&response)
                .unwrap()
                .split_whitespace()
                .nth(1),
            Some(status)
        );
        assert!(setup.forwards.try_recv().is_err());
    }
    let mut tcp = proxy_request(proxy.local_addr(), "app.home.test", &authorization).await;
    let (user, _, mut stream) = timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    assert_eq!(user, USER);
    let mut request = Vec::new();
    let mut byte = [0];
    while !request.ends_with(b"\r\n\r\n") {
        timeout(WAIT, stream.read_exact(&mut byte))
            .await
            .unwrap()
            .unwrap();
        request.push(byte[0]);
    }
    let request = String::from_utf8(request).unwrap();
    assert!(request.starts_with("GET / HTTP/1.1\r\n"));
    assert!(!request.to_ascii_lowercase().contains("proxy-authorization"));
    assert!(!request.contains(credential.password()));
    stream
        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
    let mut response = Vec::new();
    timeout(WAIT, tcp.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.ends_with(b"\r\n\r\nok"));
    proxy.shutdown().await;
    client.close();
    setup.stop().await;
}

#[tokio::test]
async fn invalid_proxy_domains_and_non_loopback_binds_are_rejected() {
    let setup = Setup::new(None).await;
    let client = setup.client().await;
    for domain in [
        "",
        ".",
        "https://home.test",
        "home.test:80",
        "*.home.test",
        "127.0.0.1",
        "-home.test",
    ] {
        assert!(
            ShareLocalProxy::bind(
                client.clone(),
                (Ipv4Addr::LOCALHOST, 0).into(),
                ProxyCredential::generate(),
                domain.into()
            )
            .await
            .is_err()
        );
    }
    assert!(
        ShareLocalProxy::bind(
            client.clone(),
            (Ipv4Addr::UNSPECIFIED, 0).into(),
            ProxyCredential::generate(),
            "home.test".into()
        )
        .await
        .is_err()
    );
    client.close();
    setup.stop().await;
}

#[tokio::test]
async fn proxy_shutdown_drops_active_forwarding() {
    let mut setup = Setup::new(None).await;
    let client = setup.client().await;
    let credential = ProxyCredential::generate();
    let authorization = format!(
        "Basic {}",
        STANDARD.encode(format!(
            "{}:{}",
            credential.username(),
            credential.password()
        ))
    );
    let proxy = ShareLocalProxy::bind(
        client.clone(),
        (Ipv4Addr::LOCALHOST, 0).into(),
        credential,
        "home.test".into(),
    )
    .await
    .unwrap();
    let _tcp = proxy_request(proxy.local_addr(), "app.home.test", &authorization).await;
    let (_, _, mut stream) = timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    // Consume the prefix before asserting reset on the next read.
    let mut byte = [0];
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        timeout(WAIT, stream.read_exact(&mut byte))
            .await
            .unwrap()
            .unwrap();
        head.push(byte[0]);
    }
    proxy.shutdown().await;
    assert!(
        timeout(WAIT, stream.read(&mut byte))
            .await
            .unwrap()
            .is_err()
    );
    client.close();
    setup.stop().await;
}
