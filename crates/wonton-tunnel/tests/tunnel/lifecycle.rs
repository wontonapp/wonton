//! 下载 EOF 后释放资源；还在双向传输的连接不受收尾期限影响。

use std::{net::Ipv4Addr, time::Duration};

use iroh::SecretKey;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use wonton_tunnel::{Allowlist, TunnelServer};

use crate::support::{
    Home, Phone, Timeouts, Upstream, WAIT, loopback_endpoint, parse_response, read_head,
};

#[tokio::test]
async fn tcp_responses_release_streams_without_waiting_for_client_fin() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let mut home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        upstream.addr,
    )
    .await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;

    completed_connections_do_not_block_new_requests(&phone).await;
    assert_eq!(upstream.hits(), 256);
    assert_eq!(
        home.server()
            .endpoint()
            .metrics()
            .socket
            .num_conns_opened
            .get(),
        1
    );

    phone.proxy.shutdown().await;
    phone.client.close();
    home.stop().await;
}

#[tokio::test]
async fn embedded_responses_release_streams_without_waiting_for_client_fin() {
    let device = SecretKey::generate();
    let endpoint = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let (server, mut listener) =
        TunnelServer::spawn_embedded(endpoint, Allowlist::new([device.public()]));
    let handler = tokio::spawn(async move {
        while let Some(mut stream) = listener.accept().await {
            let head = read_head(&mut stream).await;
            let path = head.split_whitespace().nth(1).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{path}",
                path.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        }
    });
    let phone = Phone::start(device, server.endpoint().addr(), Timeouts::Default).await;

    completed_connections_do_not_block_new_requests(&phone).await;
    assert_eq!(server.endpoint().metrics().socket.num_conns_opened.get(), 1);

    phone.proxy.shutdown().await;
    phone.client.close();
    handler.abort();
    let _ = handler.await;
    server.shutdown().await;
}

async fn completed_connections_do_not_block_new_requests(phone: &Phone) {
    phone.client.ping().await.unwrap();
    let mut clients = Vec::new();
    for index in 0..256 {
        let mut tcp = TcpStream::connect(phone.proxy.local_addr()).await.unwrap();
        let path = format!("/completed/{index}");
        tcp.write_all(format!("GET {path} HTTP/1.1\r\nhost: home\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut raw = Vec::new();
        timeout(WAIT, tcp.read_to_end(&mut raw))
            .await
            .unwrap_or_else(|_| panic!("第 {} 个请求被已完成的连接挡住", index + 1))
            .unwrap();
        let response = parse_response(std::str::from_utf8(&raw).unwrap());
        assert_eq!(response.status, 200);
        assert_eq!(response.body, path);
        clients.push(tcp);
    }
    // 这些客户端仍未发送 FIN，连通性检查也不能被它们占着的流挡住。
    timeout(WAIT, phone.client.ping()).await.unwrap().unwrap();
    drop(clients);
}

#[tokio::test]
async fn a_live_bidirectional_connection_survives_idle_and_half_close() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let device = SecretKey::generate();
    let mut home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        listener.local_addr().unwrap(),
    )
    .await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;
    let mut tcp = TcpStream::connect(phone.proxy.local_addr()).await.unwrap();
    tcp.write_all(b"up").await.unwrap();
    let (mut upstream, _) = timeout(WAIT, listener.accept()).await.unwrap().unwrap();
    let mut pair = [0; 2];
    timeout(WAIT, upstream.read_exact(&mut pair))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&pair, b"up");
    upstream.write_all(b"dn").await.unwrap();
    timeout(WAIT, tcp.read_exact(&mut pair))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&pair, b"dn");

    // 比 EOF 后收尾窗口长；没有下载 EOF 的连接不能因此被关闭。
    tokio::time::sleep(Duration::from_secs(3)).await;
    tcp.write_all(b"end").await.unwrap();
    tcp.shutdown().await.unwrap();
    let mut rest = Vec::new();
    timeout(WAIT, upstream.read_to_end(&mut rest))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rest, b"end");
    upstream.write_all(b"after client fin").await.unwrap();
    upstream.shutdown().await.unwrap();
    rest.clear();
    timeout(WAIT, tcp.read_to_end(&mut rest))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rest, b"after client fin");

    phone.proxy.shutdown().await;
    phone.client.close();
    home.stop().await;
}
