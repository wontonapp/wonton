//! 第一档端口活着但不认令牌：回退到第二档后不撤回，主连接和探针的重连次数有上限。

use std::sync::Arc;
use std::time::Duration;

use iroh::test_utils::run_relay_server;
use iroh::{EndpointAddr, SecretKey, TransportAddr, Watcher, tls::CaTlsConfig};
use wonton_tunnel::relays::{RelayTable, relay_map, spawn_tier_fallback};
use wonton_tunnel::{Allowlist, ProxyAccess, TunnelServer};

use super::{
    FALLBACK_AFTER, FLAP_WINDOW, Forwarder, SharedToken, print_net_metrics, relay_endpoint,
    spawn_relay, timeout_get, wait_disconnected, wait_reestablished, wait_relay_connected,
};
use crate::support::{Phone, Upstream, WAIT};

#[tokio::test]
async fn a_refusing_relay_bounds_main_and_probe_reconnects() -> anyhow::Result<()> {
    let (_url, refusing) = spawn_relay(Arc::new(SharedToken::new("other"))).await?;
    let backend = refusing.https_addr().expect("配置了 TLS 地址");
    let main_front = Forwarder::spawn().await?;
    main_front.set_backend(Some(backend));
    // 探针要覆盖两档被撤的中继：一个都不连上时 `home_relay_status` 是空的，修前的实现会把
    // 它们当成“没插过”每拍重插。
    let probe_front = Forwarder::spawn().await?;
    probe_front.set_backend(Some(backend));
    let second_front = Forwarder::spawn().await?;
    second_front.set_backend(Some(backend));
    // 对照：同样连这台被拒中继、但没有任何回退机制的 endpoint。它的连接全是 iroh 自己的退避和
    // 本机的网络变化报告，用来把“机器当时的噪声”从探针的连接数里除掉。
    let control_front = Forwarder::spawn().await?;
    control_front.set_backend(Some(backend));
    let (_map, healthy_url, healthy) = run_relay_server().await?;
    let single = RelayTable::parse(&format!("https://tok@{}", main_front.front)).unwrap();
    let tiered = RelayTable::parse(&format!(
        "https://tok@{};https://tok@{};{healthy_url}",
        probe_front.front, second_front.front
    ))
    .unwrap();
    assert_eq!(tiered.tier_count(), 3);
    let control_table = RelayTable::parse(&format!("https://tok@{}", control_front.front)).unwrap();
    let main = relay_endpoint(SecretKey::generate(), relay_map(&single.views(), 1)).await;
    let _main_fallback = spawn_tier_fallback(
        main.clone(),
        single,
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    let tiered_endpoint =
        relay_endpoint(SecretKey::generate(), relay_map(&tiered.views(), 1)).await;
    let control = relay_endpoint(SecretKey::generate(), relay_map(&control_table.views(), 1)).await;
    let mut fallback = spawn_tier_fallback(
        tiered_endpoint.clone(),
        tiered,
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    // 探针只在回退激活后存在；落到第三档、它开始覆盖两档以后才数。
    tokio::time::timeout(WAIT, fallback.wait_enabled_tiers(3))
        .await
        .expect("拒绝令牌应回退到可用的第三档");
    let probes = || probe_front.connections() + second_front.connections();
    print_net_metrics("start");
    // 前 5 秒有建连的突发，噪声大；断言只数第 5–15 秒的稳态。
    let mut main_baseline = 0;
    let mut probe_baseline = 0;
    let mut control_baseline = 0;
    for second in 1..=15 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if second == 5 {
            main_baseline = main_front.connections();
            probe_baseline = probes();
            control_baseline = control_front.connections();
        }
        eprintln!(
            "relay attempts second={second} main={} probes={} control={} main_status={:?}",
            main_front.connections(),
            probes(),
            control_front.connections(),
            main.home_relay_status().get()
        );
    }
    print_net_metrics("end");
    let main_connections = main_front.connections() - main_baseline;
    let probe_connections = probes() - probe_baseline;
    let control_connections = control_front.connections() - control_baseline;
    let main_total = main_front.connections();
    let probe_total = probes();
    main.close().await;
    control.close().await;
    tiered_endpoint.close().await;
    refusing.shutdown().await?;
    healthy.shutdown().await?;
    // 稳态窗口只断言上限：16 秒封顶的退避可能让这十秒里没有新拨号。全程计数另断言至少一次，
    // 确认转发器确实观察到了主 endpoint 和探针侧的连接。
    // 稳态窗口理论上最多几次：iroh 的退避从 10 ms 起、每次翻倍、封顶 16 s；封顶后每次间隔在 8–16 秒抖动，
    // 十秒窗口最多两次计划内重拨。61 上 netmon 的 LinkChangeMajor 会重置退避并立即重拨，这部分
    // 次数由环境决定，无法从参数推出。对照 endpoint 跑同一个 iroh actor、共享同一批网络事件，
    // 所以它这十秒的计数就是本轮噪声基线；主 endpoint 和它同样是单档单中继，只多给两秒采样
    // 对齐的余量。20 是安静机器上的绝对下限，基线更大时按 control + 2 放宽。
    let main_limit = (control_connections + 2).max(20);
    assert!(
        main_connections <= main_limit,
        "单档主 endpoint 稳态十秒连接数 {main_connections} 超过 {main_limit}（对照 {control_connections} + 2，下限 20）"
    );
    assert!(
        probe_connections <= 60,
        "恢复探针稳态十秒连接数 {probe_connections} 超过 60"
    );
    assert!(
        probe_connections <= 4 * control_connections.max(3),
        "恢复探针连接数 {probe_connections} 超过对照 endpoint（{control_connections}）的四倍"
    );
    assert!(main_total >= 1, "观察窗口内主 endpoint 没有连接");
    assert!(probe_total >= 1, "观察窗口内探针侧没有连接");
    Ok(())
}

#[tokio::test]
async fn a_refusing_top_tier_does_not_flap() -> anyhow::Result<()> {
    // 第一档的地址活着但不收我们：中继进程在听（端口连得上），却不认我们的令牌（配额用完、令牌
    // 被吊销）。回退到第二档以后不许撤回来：端口连得上不等于中继恢复。
    // 表里放的是转发器前门：先是认令牌的中继，回退后切成拒绝令牌的中继——同一地址换成不收我们
    // 的中继，正是 #641 的来回切场景。
    let first = Forwarder::spawn().await?;
    let first_url = first.relay_url();
    let (_first_relay_url, first_relay) = spawn_relay(Arc::new(SharedToken::new("tok"))).await?;
    first.set_backend(Some(first_relay.https_addr().expect("配置了 TLS 地址")));
    let (_second_map, second_url, second_relay) = run_relay_server().await.unwrap();
    let table = RelayTable::parse(&format!("https://tok@{};{second_url}", first.front)).unwrap();
    assert_eq!(table.tier_count(), 2);

    let device = SecretKey::generate();
    let upstream = Upstream::spawn().await;
    let server_endpoint = relay_endpoint(SecretKey::generate(), relay_map(&table.views(), 1)).await;
    let mut server_fallback = spawn_tier_fallback(
        server_endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    let server = TunnelServer::spawn(
        server_endpoint.clone(),
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let phone_endpoint = relay_endpoint(device, relay_map(&table.views(), 1)).await;
    let mut phone_fallback = spawn_tier_fallback(
        phone_endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );

    // 先经第一档连上：连接建立在第二档之前，后面断言它一直不断。
    wait_relay_connected(&phone_endpoint, &first_url).await;
    wait_relay_connected(&server_endpoint, &first_url).await;
    let address = EndpointAddr::new(server.endpoint().id()).with_relay_url(first_url.clone());
    let phone = Phone::on(phone_endpoint.clone(), address, ProxyAccess::Open).await;
    let response = timeout_get(&phone, "/via-first").await;
    assert_eq!(response.status, 200);

    // 第一档停掉（后端中继关掉；转发器还在听，像反代活着但后端没了），回退到第二档。
    phone.client.set_server_addr(
        EndpointAddr::new(server.endpoint().id())
            .with_relay_url(first_url.clone())
            .with_relay_url(second_url.clone()),
    );
    first_relay.shutdown().await.ok();
    tokio::time::timeout(WAIT, phone_fallback.wait_enabled_tiers(2))
        .await
        .expect("手机侧应加进第二档");
    tokio::time::timeout(WAIT, server_fallback.wait_enabled_tiers(2))
        .await
        .expect("服务端侧应加进第二档");
    wait_relay_connected(&phone_endpoint, &second_url).await;
    wait_relay_connected(&server_endpoint, &second_url).await;
    wait_disconnected(&phone).await;
    wait_reestablished(&phone).await;
    let response = timeout_get(&phone, "/via-second").await;
    assert_eq!(response.status, 200);
    let path = phone.client.path().expect("请求应有选中路径");
    assert_eq!(path.remote, TransportAddr::Relay(second_url.clone()));

    // 第一档的地址上换成一个不认我们令牌的中继：端口连得上（转发器在听），握手被拒。
    // 不许判成“恢复”。
    let (_refusing_url, refusing_relay) = spawn_relay(Arc::new(SharedToken::new("other"))).await?;
    first.set_backend(Some(refusing_relay.https_addr().expect("配置了 TLS 地址")));
    let started = first.connections();
    eprintln!("refusing transition initial connections={started}");
    print_net_metrics("refusing-start");

    // 撤回判定在几秒内就该发生一次（修前的 TCP 探测就是这样错判的），等一个比它宽的窗口：
    // 窗口内两侧都不许把启用档数降回 1。预期超时，真的撤回了就是来回切。
    let no_flap = tokio::time::timeout(FLAP_WINDOW, phone_fallback.wait_enabled_tiers(1)).await;
    assert!(no_flap.is_err(), "手机侧不应撤回第二档：第一档只是端口活着");
    eprintln!(
        "refusing transition after eight seconds connections={}",
        first.connections()
    );
    let no_flap = tokio::time::timeout(FLAP_WINDOW, server_fallback.wait_enabled_tiers(1)).await;
    assert!(
        no_flap.is_err(),
        "服务端侧不应撤回第二档：第一档只是端口活着"
    );
    // 连接一直不断：隧道不许在这段时间里掉过线，窗口过后同一个连接还能发请求。
    let dropped = tokio::time::timeout(FLAP_WINDOW, wait_disconnected(&phone)).await;
    assert!(dropped.is_err(), "回退期间隧道不该断开");
    let response = timeout_get(&phone, "/via-second-still").await;
    assert_eq!(response.status, 200);
    let path = phone.client.path().expect("请求应有选中路径");
    assert_eq!(path.remote, TransportAddr::Relay(second_url.clone()));
    eprintln!(
        "refusing transition final connections={} delta={}",
        first.connections(),
        first.connections() - started
    );
    print_net_metrics("refusing-end");

    phone.client.close();
    phone_endpoint.close().await;
    server_endpoint.close().await;
    server.shutdown().await;
    drop(first);
    refusing_relay.shutdown().await.ok();
    second_relay.shutdown().await.ok();
    Ok(())
}
