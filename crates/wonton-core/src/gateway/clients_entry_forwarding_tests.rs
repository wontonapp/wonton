//! 转发行为：核心与 Matrix 的路由不在这里；关入口开关立即断开已升级的连接；
//! Host 头不重复、带端口的公网与局域网形式照认。

use super::*;

#[tokio::test]
async fn core_routes_not_reachable() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    // `/_wonton/` 在插件主机名上归核心（ADR-0080）；没声明访客的插件上，访客令牌路径也是 404，不转发。
    for path in ["/_wonton/api/status", "/_wonton/guest/token"] {
        let reply = home.get(&host, path, &entry_public()).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(reply.error_kind(), Some("not_found"));
    }
    // Matrix 的客户端和联邦接口也不在这里：插件主机名上的请求只会去插件。
    for path in ["/_matrix/client/versions", "/_matrix/federation/v1/send/1"] {
        let reply = home.get(&host, path, &entry_public()).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert_eq!(
            reply.headers.get("x-backend").map(|v| v.to_str().unwrap()),
            Some("entryapp")
        );
    }
    let hits = home.upstream_hits();
    assert_eq!(hits.len(), 2);
    assert!(
        hits.iter()
            .any(|hit| hit.uri.starts_with("/_matrix/client/"))
    );
    assert!(
        hits.iter()
            .any(|hit| hit.uri.starts_with("/_matrix/federation/"))
    );
}

#[tokio::test]
async fn closing_switch_drops_open_websocket() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let (listener, addr) = bind().await;
    serve(listener, home.app.clone()).await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: echo\r\nx-wonton-entry: public\r\n\r\n"
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
    // 关开关：记录写回 + 唤醒插件表，监视任务把入口的 watch 置位，已升级的连接立即断。
    let id = crate::plugin::manifest::PluginId::parse("io.wonton.test.entry").unwrap();
    home.manager
        .clients_entry_for_test(&id, "entryapp", false, false);
    let mut closed = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut closed)).await;
    assert!(read.is_ok(), "开关关上后连接应立即断开");
}

#[tokio::test]
async fn plugin_requests_carry_no_duplicate_host_and_lan_ports_pass() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    // 带端口的形式照认：公网 443、局域网 8483。
    let server = home.server();
    for (host, entry) in [
        (format!("entryapp.{server}:443"), entry_public()),
        (format!("entryapp.lan.{server}:8483"), entry_lan()),
    ] {
        let reply = home.get(&host, "/", &entry).await;
        assert_eq!(reply.status, StatusCode::OK, "{host}");
        assert_eq!(
            reply.headers.get("x-backend").map(|v| v.to_str().unwrap()),
            Some("entryapp")
        );
    }
    assert_eq!(home.upstream_hits().len(), 2);
    let first = home.upstream_hits().remove(0);
    assert_eq!(
        first.value("x-forwarded-host"),
        Some(format!("entryapp.{server}:443").as_str())
    );
}
