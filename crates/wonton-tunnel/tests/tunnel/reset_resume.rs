//! 尝试复现恢复换连接后的 error 1：所有 HTTP 请求都经过 LocalProxy，嵌入方正常完成响应。

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use base64::{Engine, engine::general_purpose::STANDARD};
use iroh::{
    EndpointAddr, SecretKey,
    endpoint::{Connection, VarInt},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
    time::timeout,
};
use wonton_tunnel::{
    Allowlist, ERROR_HEADER, PeerStream, ProxyAccess, ProxyCredential, TunnelListener,
    TunnelProtocol, TunnelStatus,
};

use crate::{
    stall::{DROP_ALL, Flaky, PASS},
    support::{
        Phone, Response, Timeouts, WAIT, endpoint_on, loopback_endpoint, parse_response, read_head,
    },
};

#[tokio::test]
async fn queued_proxy_requests_survive_retired_embedded_connection_cleanup() {
    let device = SecretKey::generate();
    let (server, mut listener, mut accepted) = embedded_home(&device).await;
    let port = server.endpoint().bound_sockets()[0].port();
    let flaky = Flaky::start((Ipv4Addr::LOCALHOST, port).into()).await;
    let address = EndpointAddr::new(server.endpoint().id()).with_ip_addr(flaky.addr);
    let endpoint = endpoint_on((Ipv6Addr::LOCALHOST, 0).into(), device, Timeouts::Default).await;
    let credential = ProxyCredential::generate();
    let phone = Phone::on(endpoint, address, access(&credential)).await;
    let mut old_tcp = request(phone.proxy.local_addr(), &credential, "/old-sync").await;
    let (mut old_stream, path) = accept_request(&mut listener).await;
    assert_eq!(path, "/old-sync");
    let old_connection = timeout(WAIT, accepted.recv()).await.unwrap().unwrap();
    phone
        .wait_for(|status| *status == TunnelStatus::Connected)
        .await;

    flaky.drop_above(DROP_ALL);
    let mut status = phone.client.watch_status();
    let revalidating = phone.client.revalidate();
    tokio::pin!(revalidating);
    // 先轮询到 revalidate 安装共享检查，再让本地代理接受新请求；不靠 yield 猜测任务顺序。
    tokio::select! {
        biased;
        result = &mut revalidating => panic!("丢包时检查不应立即结束：{result:?}"),
        () = std::future::ready(()) => {}
    }
    let mut queued = Vec::new();
    for index in 0..8 {
        let path = format!("/queued/{index}");
        queued.push((
            request(phone.proxy.local_addr(), &credential, &path).await,
            path,
        ));
    }
    tokio::select! {
        result = &mut revalidating => panic!("恢复网络之前检查不应成功：{result:?}"),
        changed = timeout(WAIT, status.wait_for(|s| *s == TunnelStatus::Connecting)) => {
            changed.unwrap().unwrap();
        }
    }
    assert_eq!(opened_connections(&server), 1);
    flaky.drop_above(PASS);
    timeout(WAIT, &mut revalidating).await.unwrap().unwrap();
    assert_eq!(opened_connections(&server), 2);

    // 每个新流先送出部分正文，旧连接的清理发生在响应中途，完整正文能发现跨连接的误复位。
    let tail = "x".repeat(128 * 1024);
    let active = start_queued_responses(&mut listener, &mut queued, tail.len()).await;

    // 换连接时客户端的关闭帧也可能被丢包；只清理记录下来的旧服务端连接，不触碰新连接。
    old_connection.close(VarInt::from_u32(0), b"retired connection cleanup");
    let mut byte = [0; 1];
    let ended = timeout(WAIT, old_stream.read(&mut byte)).await.unwrap();
    assert!(ended.is_err(), "旧连接应已关闭：{ended:?}");
    // 只在已确认旧连接关闭后丢弃；主动丢弃活跃 PeerStream 自带的 reset(1) 不算复现。
    drop(old_stream);
    let mut old_response = Vec::new();
    timeout(WAIT, old_tcp.read_to_end(&mut old_response))
        .await
        .unwrap()
        .ok();
    assert!(old_response.is_empty(), "旧请求不能重放：{old_response:?}");

    for mut stream in active {
        stream.write_all(tail.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    }
    for (mut tcp, _) in queued {
        let mut received = Vec::new();
        timeout(WAIT, tcp.read_to_end(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, tail.as_bytes());
    }
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
    assert_eq!(opened_connections(&server), 2);
    phone.client.close();
    phone.proxy.shutdown().await;
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_second_connection_from_the_same_device_keeps_the_first_active_stream_usable() {
    let device = SecretKey::generate();
    let (server, mut listener, _accepted) = embedded_home(&device).await;
    let endpoint = loopback_endpoint(device.clone(), 0, Timeouts::Default).await;
    let credential = ProxyCredential::generate();
    let first = Phone::on(
        endpoint.clone(),
        server.endpoint().addr(),
        access(&credential),
    )
    .await;
    let old_tcp = request(first.proxy.local_addr(), &credential, "/first-active").await;
    let (mut old_stream, path) = accept_request(&mut listener).await;
    assert_eq!(path, "/first-active");
    assert_eq!(old_stream.remote_id(), device.public());

    let second = Phone::on(endpoint, server.endpoint().addr(), access(&credential)).await;
    let new_tcp = request(second.proxy.local_addr(), &credential, "/second-active").await;
    let (mut new_stream, path) = accept_request(&mut listener).await;
    assert_eq!(path, "/second-active");
    assert_eq!(new_stream.remote_id(), device.public());
    assert_eq!(opened_connections(&server), 2);

    begin_response(&mut new_stream, "new-before-new-after".len()).await;
    new_stream.write_all(b"new-before-").await.unwrap();
    // 新连接已经确认并在回响应时，旧连接上的活跃请求仍可完成；正常 shutdown 后才回收旧流。
    respond(&mut old_stream, "first-still-usable").await;
    assert_response(old_tcp, "first-still-usable").await;
    drop(old_stream);
    first.client.close();
    new_stream.write_all(b"new-after").await.unwrap();
    new_stream.shutdown().await.unwrap();
    assert_response(new_tcp, "new-before-new-after").await;
    assert_eq!(second.client.status(), TunnelStatus::Connected);
    assert_eq!(opened_connections(&server), 2);
    second.client.close();
    first.proxy.shutdown().await;
    second.proxy.shutdown().await;
    server.shutdown().await.unwrap();
}

async fn start_queued_responses(
    listener: &mut TunnelListener,
    queued: &mut [(TcpStream, String)],
    remaining: usize,
) -> Vec<PeerStream> {
    let mut active = Vec::new();
    let mut seen = [false; 8];
    for _ in 0..queued.len() {
        let (mut stream, path) = accept_request(listener).await;
        let index = path
            .strip_prefix("/queued/")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert!(index < seen.len(), "意外的请求：{path}");
        assert!(!seen[index], "请求被重放：{path}");
        seen[index] = true;
        begin_response(&mut stream, path.len() + remaining).await;
        stream.write_all(path.as_bytes()).await.unwrap();
        active.push(stream);
    }
    for (tcp, expected_path) in queued {
        let head = timeout(WAIT, read_head(tcp)).await.unwrap();
        let response = parse_response(&head);
        assert_eq!(response.status, 200);
        assert_eq!(response.header(ERROR_HEADER), None);
        let mut prefix = vec![0; expected_path.len()];
        timeout(WAIT, tcp.read_exact(&mut prefix))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prefix, expected_path.as_bytes());
    }
    active
}

fn opened_connections(server: &Router) -> u64 {
    server.endpoint().metrics().socket.num_conns_opened.get()
}

#[derive(Debug)]
struct RecordConnections {
    protocol: TunnelProtocol,
    connections: mpsc::UnboundedSender<Connection>,
}

impl ProtocolHandler for RecordConnections {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.connections.send(conn.clone()).ok();
        self.protocol.accept(conn).await
    }
}

async fn embedded_home(
    device: &SecretKey,
) -> (Router, TunnelListener, mpsc::UnboundedReceiver<Connection>) {
    let endpoint = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let (protocol, listener) = TunnelProtocol::embedded(Allowlist::new([device.public()]));
    let (connections, accepted) = mpsc::unbounded_channel();
    let handler = RecordConnections {
        protocol,
        connections,
    };
    let router = Router::builder(endpoint)
        .accept(wonton_tunnel::ALPN, handler)
        .spawn();
    (router, listener, accepted)
}

fn access(credential: &ProxyCredential) -> ProxyAccess {
    ProxyAccess::Credential {
        credential: credential.clone(),
        domain: "wonton.localhost".to_owned(),
    }
}

async fn request(proxy: SocketAddr, credential: &ProxyCredential, path: &str) -> TcpStream {
    let mut tcp = TcpStream::connect(proxy).await.unwrap();
    let pair = format!("{}:{}", credential.username(), credential.password());
    let authorization = STANDARD.encode(pair);
    let connect = format!(
        "CONNECT home.wonton.localhost:80 HTTP/1.1\r\nHost: home.wonton.localhost:80\r\n\
         Proxy-Authorization: Basic {authorization}\r\n\r\n"
    );
    tcp.write_all(connect.as_bytes()).await.unwrap();
    // CONNECT 的 200 在开隧道流之前返回，能确认每条本地连接已被代理接收，不等恢复完成。
    let established = timeout(WAIT, read_head(&mut tcp)).await.unwrap();
    assert_eq!(established, "HTTP/1.1 200 Connection Established\r\n\r\n");
    let head = format!("GET {path} HTTP/1.1\r\nhost: home\r\nconnection: close\r\n\r\n");
    tcp.write_all(head.as_bytes()).await.unwrap();
    tcp
}

async fn accept_request(listener: &mut TunnelListener) -> (PeerStream, String) {
    let mut stream = timeout(WAIT, listener.accept()).await.unwrap().unwrap();
    let head = timeout(WAIT, read_head(&mut stream)).await.unwrap();
    let path = head.split_whitespace().nth(1).unwrap().to_owned();
    (stream, path)
}

async fn begin_response(stream: &mut PeerStream, length: usize) {
    let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {length}\r\nconnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
}

async fn respond(stream: &mut PeerStream, body: &str) {
    begin_response(stream, body.len()).await;
    stream.write_all(body.as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
}

async fn assert_response(mut tcp: TcpStream, body: &str) {
    let mut raw = Vec::new();
    timeout(WAIT, tcp.read_to_end(&mut raw))
        .await
        .unwrap()
        .unwrap();
    let response: Response = parse_response(std::str::from_utf8(&raw).unwrap());
    assert_eq!(response.status, 200);
    assert_eq!(response.header(ERROR_HEADER), None);
    assert_eq!(response.body, body);
}
