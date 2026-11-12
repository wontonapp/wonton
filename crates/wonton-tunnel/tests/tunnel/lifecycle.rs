//! 下载 EOF 后释放资源；还在双向传输的连接不受收尾期限影响；流额度用完时请求不无限等。

use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};

use iroh::SecretKey;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use wonton_tunnel::{Allowlist, TunnelServer};

use crate::support::{
    Home, Phone, Timeouts, Upstream, WAIT, loopback_endpoint, parse_response, read_head,
};

/// 测试用的上游：`/hold/` 开头的请求一直不回，其余的回 200 后关闭连接。
struct HoldingUpstream {
    addr: SocketAddr,
    accepted: watch::Sender<usize>,
    task: JoinHandle<()>,
}

impl HoldingUpstream {
    async fn spawn() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (accepted, _) = watch::channel(0usize);
        let count = accepted.clone();
        let task = tokio::spawn(async move {
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                count.send_modify(|count| *count += 1);
                tokio::spawn(serve_holding(tcp));
            }
        });
        Self {
            addr,
            accepted,
            task,
        }
    }

    async fn wait_for(&self, expected: usize) {
        let mut accepted = self.accepted.subscribe();
        timeout(WAIT, accepted.wait_for(|count| *count >= expected))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "上游只接受了 {} 条连接，要 {expected}",
                    *self.accepted.borrow()
                )
            })
            .unwrap();
    }
}

impl Drop for HoldingUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_holding(mut tcp: TcpStream) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        // 换连接时旧连接上的流被掐断，上游会看到请求头没发完就关闭。
        if !matches!(tcp.read(&mut byte).await, Ok(1)) {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    if path.starts_with("/hold/") {
        std::future::pending::<()>().await;
    }
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{path}",
        path.len()
    );
    tcp.write_all(response.as_bytes()).await.unwrap();
    tcp.shutdown().await.unwrap();
}

/// 服务端每条连接放的流数（`server.rs` 的 `MAX_CONCURRENT_STREAMS`）。
const STREAM_QUOTA: usize = 256;

/// 一条连接的流额度被占满时，新请求不无限等：3 秒内换一条新连接把它送出去。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_after_the_stream_quota_is_used_up_goes_out_on_a_new_connection() {
    let upstream = HoldingUpstream::spawn().await;
    let device = SecretKey::generate();
    let mut home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        upstream.addr,
    )
    .await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;

    // 不先发别的请求：服务端攒够 1/8 的额度才通知客户端，先结束的流腾出的额度不会马上回来。
    let mut held = JoinSet::new();
    for index in 0..STREAM_QUOTA {
        let proxy = phone.proxy.local_addr();
        held.spawn(async move {
            let mut tcp = TcpStream::connect(proxy).await.unwrap();
            let request = format!("GET /hold/{index} HTTP/1.1\r\nhost: home\r\n\r\n");
            tcp.write_all(request.as_bytes()).await.unwrap();
            std::future::pending::<()>().await;
        });
    }
    upstream.wait_for(STREAM_QUOTA).await;
    assert_eq!(phone.client.stats().streams.forward, STREAM_QUOTA);

    let result = timeout(Duration::from_secs(8), phone.get("/after")).await;
    let opened = home
        .server()
        .endpoint()
        .metrics()
        .socket
        .num_conns_opened
        .get();
    held.abort_all();
    while held.join_next().await.is_some() {}
    phone.proxy.shutdown().await;
    phone.client.close();
    home.stop().await;

    let response = result.expect("流额度用完后新请求一直没回");
    assert_eq!(response.status, 200);
    assert_eq!(opened, 2);
}

/// 一个接一个的短请求，每个结束后它的流就还回去：开着的流数不随请求数增长，超过一条连接的额度也不卡。
#[tokio::test]
async fn sequential_requests_beyond_the_quota_do_not_pile_up_streams() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let mut home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        upstream.addr,
    )
    .await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;

    for index in 0..STREAM_QUOTA + 44 {
        let path = format!("/short/{index}");
        let response = phone.get(&path).await;
        assert_eq!(response.body, path);
    }
    let streams = phone.client.stats().streams;
    assert!(streams.peak <= 2, "{streams:?}");

    phone.proxy.shutdown().await;
    phone.client.close();
    home.stop().await;
}

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
