//! 入口准入与请求校验：只有入口容器的对端能进；入口头必须与主机名对上；开关关闭与
//! 未知标签的拒绝；点段路径在到后端之前 400。

use super::*;

#[tokio::test]
async fn only_entry_container_peer_accepted() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let server = home.server();
    let app =
        crate::entry_runtime::router_with_trusted(home.gateway.clone(), "127.0.0.1:9".to_owned());
    let ask = |peer: std::net::IpAddr, host: String| {
        let app = app.clone();
        async move {
            let mut request = Request::builder()
                .method(Method::GET)
                .uri("/.well-known/matrix/server")
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::new(peer, 40000)));
            Reply::of(app.oneshot(request).await.unwrap()).await
        }
    };
    // 入口容器的地址：过了对端检查，联邦照常回答。
    let trusted = ask(std::net::IpAddr::from([127, 0, 0, 1]), server.clone()).await;
    assert_eq!(trusted.status, StatusCode::OK);
    // 别的地址（被攻破的连接器、宿主）在这个监听上一个字节都得不到。
    let stranger = ask(
        std::net::IpAddr::from([10, 9, 9, 9]),
        format!("entryapp.{server}"),
    )
    .await;
    assert_eq!(stranger.status, StatusCode::FORBIDDEN);
    assert_eq!(stranger.json()["errcode"], "M_FORBIDDEN");
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn missing_or_double_or_mismatched_entry_header_rejected() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    let public = home.host("entryapp");
    let lan = home.lan_host("entryapp");
    let no_header = home.get(&public, "/", &[]).await;
    assert_eq!(no_header.status, StatusCode::BAD_REQUEST);
    assert_eq!(no_header.error_kind(), Some("invalid_input"));
    let doubled = home
        .get(
            &public,
            "/",
            &[("x-wonton-entry", "public"), ("x-wonton-entry", "lan")],
        )
        .await;
    assert_eq!(doubled.status, StatusCode::BAD_REQUEST);
    let mismatched = home.get(&public, "/", &[("x-wonton-entry", "lan")]).await;
    assert_eq!(mismatched.status, StatusCode::BAD_REQUEST);
    let mismatched_lan = home.get(&lan, "/", &[("x-wonton-entry", "public")]).await;
    assert_eq!(mismatched_lan.status, StatusCode::BAD_REQUEST);
    let wrong_value = home.get(&public, "/", &[("x-wonton-entry", "probe")]).await;
    assert_eq!(wrong_value.status, StatusCode::BAD_REQUEST);
    assert!(home.upstream_hits().is_empty());
    // 两个开关都开着、头也对得上时才转发。
    let ok = home.get(&public, "/", &entry_public()).await;
    assert_eq!(ok.status, StatusCode::OK);
    let ok_lan = home.get(&lan, "/", &entry_lan()).await;
    assert_eq!(ok_lan.status, StatusCode::OK);
}

#[tokio::test]
async fn closed_switch_rejected() {
    let home = Home::start().await;
    let id = crate::plugin::manifest::PluginId::parse("io.wonton.test.entry").unwrap();
    home.register(ENTRY_APP, "entryapp", false, false);
    let host = home.host("entryapp");
    let closed = home.get(&host, "/", &entry_public()).await;
    assert_eq!(closed.status, StatusCode::FORBIDDEN);
    assert_eq!(closed.error_kind(), Some("entry_closed"));
    assert!(home.upstream_hits().is_empty());
    // 开关是记录里的：写上再唤醒插件表，下一个请求就能过。
    home.manager
        .clients_entry_for_test(&id, "entryapp", true, true);
    let opened = home.get(&host, "/", &entry_public()).await;
    assert_eq!(opened.status, StatusCode::OK);
    assert_eq!(home.upstream_hits().len(), 1);
}

#[tokio::test]
async fn unknown_label_421() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let server = home.server();
    for host in [
        format!("ghost.{server}"),
        format!("a.b.{server}"),
        format!("{server}.evil.test"),
        format!("{server}:12345"),
    ] {
        let reply = home.get(&host, "/", &entry_public()).await;
        assert_eq!(reply.status, StatusCode::MISDIRECTED_REQUEST, "{host}");
        assert_eq!(reply.json()["errcode"], "M_UNRECOGNIZED");
    }
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn dot_segments_400_before_upstream() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    let public = home.host("entryapp");
    for path in ["/a/../b", "/a/%2e%2e/b", "/..%2fadmin", "/a/..;x/b"] {
        let reply = home.get(&public, path, &entry_public()).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(reply.error_kind(), Some("invalid_input"));
    }
    assert!(home.upstream_hits().is_empty());
    // 点段在两种入口都拒（review P1-6 裁定）：应用会各自解成点段，局域网的失败计数同样不能被绕过。
    let lan = home.lan_host("entryapp");
    let forwarded = home.get(&lan, "/a/../b", &entry_lan()).await;
    assert_eq!(forwarded.status, StatusCode::BAD_REQUEST);
    assert_eq!(forwarded.error_kind(), Some("invalid_input"));
    assert!(home.upstream_hits().is_empty());
}
