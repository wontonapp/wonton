//! 连接方式的行为测试：https 连检查过的 IP 但 SNI 和 `Host` 是原域名；经主人的代理时代理收到的是 IP，不是域名。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::get;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use wonton_protocol::LinkPreviewFailure;

use super::Limits;
use super::test_support::{SITE_IP, og_page, world, world_with};
use crate::plugin::egress::address::ProxyAddress;
use crate::plugin::languages::Languages;

/// 读到请求头结束（空行）为止，返回读到的文字。
async fn read_head<S: AsyncReadExt + Unpin>(stream: &mut S) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            break;
        }
        head.push(byte[0]);
    }
    String::from_utf8_lossy(&head).into_owned()
}

fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

/// 证书只给 `name` 的 https 站点，记下每个连接的 SNI 和 `Host`。
async fn tls_site(
    name: &str,
) -> (
    super::test_support::TlsSite,
    Arc<Mutex<Vec<(String, String)>>>,
) {
    let key = rcgen::generate_simple_self_signed(vec![name.to_owned()]).unwrap();
    let cert = key.cert.der().clone();
    let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let server = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], private)
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let acceptor = TlsAcceptor::from(Arc::new(server));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, record) = (acceptor.clone(), record.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let sni = tls.get_ref().1.server_name().unwrap_or_default().to_owned();
                let head = read_head(&mut tls).await;
                let host = header_value(&head, "host").unwrap_or_default();
                record.lock().unwrap().push((sni, host));
                let body = og_page("Secure", None);
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(reply.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    ((addr, Arc::new(client)), seen)
}

#[tokio::test]
async fn https_connects_to_the_checked_ip_but_keeps_the_domain_for_sni_and_host() {
    let (site, seen) = tls_site("secure.test").await;
    let dns: &[(&str, &[&str])] = &[("secure.test", &[SITE_IP]), ("other.test", &[SITE_IP])];
    let (previews, world) = world_with(Router::new(), dns, Limits::default(), Some(site)).await;
    for (url, host) in [
        ("https://secure.test/", "secure.test"),
        ("https://secure.test:8443/a", "secure.test:8443"),
    ] {
        let preview = previews.preview(url, "device").await.unwrap();
        assert_eq!(preview.title.as_deref(), Some("Secure"));
        let last = seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last, ("secure.test".to_owned(), host.to_owned()));
    }
    let site_ip: std::net::IpAddr = SITE_IP.parse().unwrap();
    assert!(
        world
            .dialed
            .lock()
            .unwrap()
            .iter()
            .all(|a| a.ip() == site_ip)
    );
    // 证书对不上域名就不认：TLS 校验没有因为按 IP 连而关掉。
    let wrong = previews.preview("https://other.test/", "device").await;
    assert_eq!(wrong.unwrap_err().kind, LinkPreviewFailure::UpstreamError);
}

/// HTTP 代理替身：记下 `CONNECT` 请求头，然后接到假站点。
async fn connect_proxy(site: SocketAddr) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            let head = read_head(&mut client).await;
            record.lock().unwrap().push(head);
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .unwrap();
            let mut upstream = TcpStream::connect(site).await.unwrap();
            tokio::spawn(async move {
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });
    (port, seen)
}

/// SOCKS5 代理替身（不要认证）：记下请求里的目标，只接受 IPv4 地址，域名一律回失败。
async fn socks_proxy(site: SocketAddr) -> (u16, Arc<Mutex<Vec<SocketAddr>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            let mut greeting = [0u8; 2];
            client.read_exact(&mut greeting).await.unwrap();
            let mut methods = vec![0u8; greeting[1] as usize];
            client.read_exact(&mut methods).await.unwrap();
            client.write_all(&[5, 0]).await.unwrap();
            let mut request = [0u8; 4];
            client.read_exact(&mut request).await.unwrap();
            if request[3] != 1 {
                let _ = client.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                continue;
            }
            let mut target = [0u8; 6];
            client.read_exact(&mut target).await.unwrap();
            let ip = std::net::Ipv4Addr::new(target[0], target[1], target[2], target[3]);
            let to = SocketAddr::new(ip.into(), u16::from_be_bytes([target[4], target[5]]));
            record.lock().unwrap().push(to);
            client
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
            let mut upstream = TcpStream::connect(site).await.unwrap();
            tokio::spawn(async move {
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });
    (port, seen)
}

fn use_proxy(proxy: &crate::plugin::egress::core_proxy::CoreProxy, url: &str) {
    proxy.set(Some(
        &ProxyAddress::parse(url, &Languages::default()).unwrap(),
    ));
}

fn page_routes() -> Router {
    Router::new().route(
        "/",
        get(|| async { axum::response::Html(og_page("Proxied", None)).into_response() }),
    )
}

#[tokio::test]
async fn the_owner_http_proxy_is_asked_for_the_ip_with_the_password() {
    let dns: &[(&str, &[&str])] = &[("loopback.test", &["127.0.0.1"])];
    let (previews, world) = world_with(page_routes(), dns, Limits::default(), None).await;
    let (port, seen) = connect_proxy(world.site).await;
    use_proxy(&world.proxy, &format!("http://me:s3cret@127.0.0.1:{port}"));

    let preview = previews
        .preview("http://site.test/", "device")
        .await
        .unwrap();
    assert_eq!(preview.title.as_deref(), Some("Proxied"));
    let head = seen.lock().unwrap().first().cloned().unwrap();
    assert!(
        head.starts_with(&format!("CONNECT {SITE_IP}:80 ")),
        "{head}"
    );
    // me:s3cret 的 base64。
    assert_eq!(
        header_value(&head, "proxy-authorization").as_deref(),
        Some("Basic bWU6czNjcmV0")
    );
    assert!(world.dialed.lock().unwrap().is_empty(), "经代理时不直连");
    assert_eq!(world.hits.lock().unwrap().as_slice(), ["site.test /"]);

    // 内网地址在交给代理之前就拒绝了。
    let blocked = previews.preview("http://loopback.test/", "device").await;
    assert_eq!(
        blocked.unwrap_err().kind,
        LinkPreviewFailure::BlockedAddress
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_owner_socks5_proxy_gets_the_ip_not_the_name() {
    let (previews, world) = world(page_routes()).await;
    let (port, seen) = socks_proxy(world.site).await;
    use_proxy(&world.proxy, &format!("socks5://127.0.0.1:{port}"));

    let preview = previews
        .preview("http://site.test/", "device")
        .await
        .unwrap();
    assert_eq!(preview.title.as_deref(), Some("Proxied"));
    let expected: SocketAddr = format!("{SITE_IP}:80").parse().unwrap();
    assert_eq!(seen.lock().unwrap().as_slice(), [expected]);
    assert!(world.dialed.lock().unwrap().is_empty());
}
