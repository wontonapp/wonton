use super::*;
use crate::ProxyCredential;
use base64::{Engine, engine::general_purpose::STANDARD};
use tokio::net::TcpStream;

async fn request_hub(
    registration: &ProxyRegistration,
    host: &str,
    auth: Option<&str>,
) -> TcpStream {
    let mut tcp = TcpStream::connect(registration.local_addr()).await.unwrap();
    let credential = registration.credential();
    let authorization = auth.map(str::to_owned).unwrap_or_else(|| {
        format!(
            "Basic {}",
            STANDARD.encode(format!(
                "{}:{}",
                credential.username(),
                credential.password()
            ))
        )
    });
    tcp.write_all(format!("GET http://{host}/ HTTP/1.1\r\nHost: {host}\r\nProxy-Authorization: {authorization}\r\n\r\n")
        .as_bytes()).await.unwrap();
    tcp
}

async fn consume_request(stream: &mut ShareStream) -> String {
    let mut data = Vec::new();
    let mut byte = [0];
    while !data.ends_with(b"\r\n\r\n") {
        timeout(WAIT, stream.read_exact(&mut byte))
            .await
            .unwrap()
            .unwrap();
        data.push(byte[0]);
    }
    String::from_utf8(data).unwrap()
}

async fn downstream(setup: &Setup) -> (ShareClient, ShareLocalProxy) {
    let client = setup.client().await;
    let proxy = ShareLocalProxy::bind(
        client.clone(),
        (Ipv4Addr::LOCALHOST, 0).into(),
        ProxyCredential::generate(),
        "home.test".into(),
    )
    .await
    .unwrap();
    (client, proxy)
}

#[tokio::test]
async fn same_host_with_distinct_credentials_routes_to_distinct_source_connections() {
    let mut first = Setup::new(None).await;
    let mut second = Setup::new(None).await;
    let (client_a, proxy_a) = downstream(&first).await;
    let (client_b, proxy_b) = downstream(&second).await;
    let hub = ProxyHub::bind().await.unwrap();
    let a = hub
        .register(
            proxy_a.local_addr(),
            proxy_a.credential().clone(),
            "home.test".into(),
        )
        .unwrap();
    let b = hub
        .register(
            proxy_b.local_addr(),
            proxy_b.credential().clone(),
            "home.test".into(),
        )
        .unwrap();
    assert_eq!(a.local_addr(), b.local_addr());
    assert_ne!(a.credential(), b.credential());
    let mut tcp_a = request_hub(&a, "app.home.test", None).await;
    let (_, _, mut stream_a) = timeout(WAIT, first.forwards.recv()).await.unwrap().unwrap();
    assert!(second.forwards.try_recv().is_err());
    let mut tcp_b = request_hub(&b, "app.home.test", None).await;
    let (_, _, mut stream_b) = timeout(WAIT, second.forwards.recv())
        .await
        .unwrap()
        .unwrap();
    for (stream, marker, registration) in [(&mut stream_a, "a", &a), (&mut stream_b, "b", &b)] {
        let request = consume_request(stream).await;
        assert!(!request.to_ascii_lowercase().contains("proxy-authorization"));
        assert!(!request.contains(registration.credential().password()));
        assert!(!request.contains(proxy_a.credential().password()));
        assert!(!request.contains(proxy_b.credential().password()));
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: 1\r\nconnection: close\r\n\r\n{marker}"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    }
    let mut a_bytes = Vec::new();
    let mut b_bytes = Vec::new();
    timeout(WAIT, tcp_a.read_to_end(&mut a_bytes))
        .await
        .unwrap()
        .unwrap();
    timeout(WAIT, tcp_b.read_to_end(&mut b_bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(a_bytes.ends_with(b"\r\n\r\na"));
    assert!(b_bytes.ends_with(b"\r\n\r\nb"));
    drop(a);
    drop(b);
    drop(hub);
    proxy_a.shutdown().await;
    proxy_b.shutdown().await;
    client_a.close();
    client_b.close();
    first.stop().await;
    second.stop().await;
}

#[tokio::test]
async fn hub_rejects_unknown_mismatched_credentials_and_outside_domains() {
    let mut setup = Setup::new(None).await;
    let (client, proxy) = downstream(&setup).await;
    let hub = ProxyHub::bind().await.unwrap();
    let route = hub
        .register(
            proxy.local_addr(),
            proxy.credential().clone(),
            "home.test".into(),
        )
        .unwrap();
    let mismatch = format!(
        "Basic {}",
        STANDARD.encode(format!("other:{}", route.credential().password()))
    );
    for (host, authorization, expected) in [
        ("app.home.test", Some("Basic unknown"), "407"),
        ("app.home.test", Some(mismatch.as_str()), "407"),
        ("app.home.test.evil.test", None, "403"),
    ] {
        let mut tcp = request_hub(&route, host, authorization).await;
        let mut bytes = Vec::new();
        timeout(WAIT, tcp.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&bytes)
                .unwrap()
                .split_whitespace()
                .nth(1),
            Some(expected)
        );
        assert!(setup.forwards.try_recv().is_err());
    }
    drop(route);
    drop(hub);
    proxy.shutdown().await;
    client.close();
    setup.stop().await;
}

#[tokio::test]
async fn unregister_interrupts_active_source_and_rejects_its_old_credential() {
    let mut setup = Setup::new(None).await;
    let (client, proxy) = downstream(&setup).await;
    let hub = ProxyHub::bind().await.unwrap();
    let route = hub
        .register(
            proxy.local_addr(),
            proxy.credential().clone(),
            "home.test".into(),
        )
        .unwrap();
    let old = format!(
        "Basic {}",
        STANDARD.encode(format!(
            "{}:{}",
            route.credential().username(),
            route.credential().password()
        ))
    );
    let _tcp = request_hub(&route, "app.home.test", None).await;
    let (_, _, mut stream) = timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    consume_request(&mut stream).await;
    drop(route);
    let mut byte = [0];
    let read = timeout(WAIT, stream.read(&mut byte)).await.unwrap();
    assert!(matches!(read, Ok(0) | Err(_)));
    let fresh = hub
        .register(
            proxy.local_addr(),
            proxy.credential().clone(),
            "home.test".into(),
        )
        .unwrap();
    let mut tcp = request_hub(&fresh, "app.home.test", Some(&old)).await;
    let mut bytes = Vec::new();
    timeout(WAIT, tcp.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .split_whitespace()
            .nth(1),
        Some("407")
    );
    assert!(setup.forwards.try_recv().is_err());
    drop(fresh);
    drop(hub);
    proxy.shutdown().await;
    client.close();
    setup.stop().await;
}
