//! 和家里服务器连接的行为测试：只经由导出给原生壳的接口（`ClientCore`、`Action`、订阅）驱动。
//! 隧道服务端、插件上游都跑在本机，只走回环地址，不依赖外网。

mod migration;
mod support;

use support::{BEFORE_IDLE_TIMEOUT, WEB_URL, get, wait_until_ticket_has};
pub(crate) use support::{Home, Phone};

use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::{Duration, Instant};

use iroh::EndpointId;
use wonton_client::{
    Action, ConnectionNotice, CoreError, DeviceNetwork, HomeFailure, HomeStatus, PushMethod,
};

#[tokio::test]
async fn a_device_keeps_its_identity_and_waits_for_connection_info() {
    let dir = tempfile::tempdir().unwrap();
    let first = Phone::open(dir.path()).home();
    assert_eq!(first.status, HomeStatus::NotConfigured);
    assert!(first.proxy.is_none());
    assert!(EndpointId::from_str(&first.device_id).is_ok(), "{first:?}");

    let again = Phone::open(dir.path()).home();
    assert_eq!(again.device_id, first.device_id, "设备身份要跨重启保持");
}

#[tokio::test]
async fn invalid_connection_info_is_rejected_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let before = phone.home();
    for ticket in ["", "{}", "not json"] {
        let result = phone.dispatch(Action::ConnectHome {
            ticket: ticket.into(),
        });
        assert!(
            matches!(result, Err(CoreError::InvalidInput { .. })),
            "{ticket}"
        );
    }
    assert_eq!(phone.home(), before);
}

#[tokio::test]
async fn a_paired_device_connects_and_only_this_app_can_use_its_proxy() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;

    phone.connect(&home);
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    assert_eq!(
        view.server_id,
        Some(home.server().endpoint().id().to_string())
    );
    assert_eq!(view.web_url.as_deref(), Some(WEB_URL));
    assert_eq!(view.path.map(|path| path.relay), Some(false));

    let proxy = view.proxy.unwrap();
    assert_eq!(proxy.domain, "wonton.localhost");
    let (status, body) = get(&proxy, WEB_URL, "/hello", true).await;
    assert_eq!((status, body.as_str()), (200, "/hello"));
    let (status, _) = get(&proxy, WEB_URL, "/hello", false).await;
    assert_eq!(status, 407, "不带会话凭据的请求（同机其他 App）要被拒");
    let (status, _) = get(&proxy, "http://cdn.example.com", "/app.js", true).await;
    assert_eq!(status, 403, "家里服务器以外的地址不进隧道");
}

#[tokio::test]
async fn sustained_traffic_does_not_publish_a_home_view_for_each_request() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.stop_periodic_checks();
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .dispatch(Action::NetworkChanged {
            network: DeviceNetwork::Wifi,
        })
        .unwrap();
    let before = phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.proxy.is_some()
                && view.path.is_some()
                && view.details.network == Some(DeviceNetwork::Wifi)
        })
        .await;
    let proxy = before.proxy.unwrap();
    let updates = phone.subscribe_home();
    updates.next().await.unwrap();
    let started = Instant::now();
    let traffic = async {
        for _ in 0..300 {
            let (status, body) = get(&proxy, WEB_URL, "/traffic", true).await;
            assert_eq!((status, body.as_str()), (200, "/traffic"));
            // 同一网络的重复通知要求重算，但状态、路径类型和提示都没有变。
            phone
                .dispatch(Action::NetworkChanged {
                    network: DeviceNetwork::Wifi,
                })
                .unwrap();
        }
    };
    tokio::pin!(traffic);
    let mut publications = 0;
    loop {
        tokio::select! {
            () = &mut traffic => break,
            view = updates.next() => {
                let view = view.expect("连接订阅提前结束");
                assert_eq!(view.status, HomeStatus::Connected);
                publications += 1;
            }
        }
    }
    let elapsed = started.elapsed();
    let limit = elapsed.as_secs() + 3;
    eprintln!("300 requests: {publications} home publications in {elapsed:?}, limit {limit}");
    assert!(publications <= limit, "{publications} > {limit}");
    phone
        .wait_for(|view| view.details.sent_packets > before.details.sent_packets)
        .await;
}

#[tokio::test]
async fn an_unregistered_device_is_told_it_needs_pairing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[]).await;

    phone.connect(&home);
    let view = phone
        .wait_for(|view| matches!(view.status, HomeStatus::Disconnected { .. }))
        .await;
    assert!(
        matches!(
            view.status,
            HomeStatus::Disconnected {
                reason: HomeFailure::Unauthorized,
                ..
            }
        ),
        "{view:?}"
    );
    assert_eq!(view.notice, Some(ConnectionNotice::DeviceRejected));
}

#[tokio::test]
async fn an_unregistered_device_stops_trying_until_it_is_paired() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| matches!(view.status, HomeStatus::Disconnected { .. }))
        .await;
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    let before = opened.get();

    // 比定时检查的间隔长：重试也没用，状态也不该在“连接中”和“要配对”之间跳。
    tokio::time::sleep(Duration::from_secs(7)).await;

    assert_eq!(opened.get(), before);
    assert!(matches!(
        phone.home().status,
        HomeStatus::Disconnected {
            reason: HomeFailure::Unauthorized,
            ..
        }
    ));
}

#[tokio::test]
async fn the_server_is_remembered_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    drop(phone);

    let restarted = Phone::open(dir.path());
    let view = restarted
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let (status, _) = get(&view.proxy.unwrap(), WEB_URL, "/again", true).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn a_server_that_moved_is_reached_at_its_new_address_without_pairing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    let moved = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap();
    home.report_address(moved);
    phone.connect(&home);
    wait_until_ticket_has(dir.path(), moved).await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    home.stop().await;
    home.start_again(moved.port()).await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    home.wait_for_a_connection().await;
    drop(phone);

    let restarted = Phone::open(dir.path());
    restarted
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
}

/// 取地址失败一次（换网、机器忙）：自动补取，过时标记先真后假，连接不用重建。
#[tokio::test]
async fn a_failed_address_refresh_is_retried_and_clears_the_stale_mark() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let lan: SocketAddr = "192.168.1.2:18482".parse().unwrap();
    let public: SocketAddr = "203.0.113.2:18482".parse().unwrap();
    *home.tunnel_info.lock().unwrap() = Some(
        serde_json::json!({
            "endpoint_id": home.key.public(),
            "relays": [],
            "direct_addrs": [lan, public],
        })
        .to_string(),
    );
    home.fail_next_tunnel(1);
    phone.connect(&home);

    let stale = phone.wait_for(|view| view.details.addresses_stale).await;
    assert_eq!(stale.status, HomeStatus::Connected);

    wait_until_ticket_has(dir.path(), public).await;
    let view = phone.wait_for(|view| !view.details.addresses_stale).await;
    assert_eq!(view.status, HomeStatus::Connected);
    assert_eq!(
        home.server()
            .endpoint()
            .metrics()
            .socket
            .num_conns_opened
            .get(),
        1,
        "补取不该重新建连"
    );
}

/// 一直取不到：过时标记一直亮，连接和其它请求照常，退避期间也不会把连接拖下来。
#[tokio::test]
async fn a_failing_address_refresh_keeps_the_connection_usable() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.report_address("203.0.113.2:18482".parse().unwrap());
    home.fail_next_tunnel(usize::MAX);
    phone.connect(&home);

    let view = phone.wait_for(|view| view.details.addresses_stale).await;
    assert_eq!(view.status, HomeStatus::Connected);
    let proxy = view.proxy.clone().unwrap();
    let (status, body) = get(&proxy, WEB_URL, "/while-stale", true).await;
    assert_eq!((status, body.as_str()), (200, "/while-stale"));

    // 退避是 1、2、4 秒：这段时间里又取过两次，标记不能自己掉。
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let view = phone.home();
    assert!(view.details.addresses_stale, "{view:?}");
    assert_eq!(view.status, HomeStatus::Connected);
    let (status, _) = get(&proxy, WEB_URL, "/still-stale", true).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn coming_back_to_the_foreground_reconnects() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    // App 在后台时服务端重启了，回到前台后手机连上重启后的服务端。不等状态里的 Disconnected：状态经 watch 通道推送、
    // 只留最新值，重连得快时那一下断开会被合并掉，机器忙的时候测试就等不到它。看的是重启后的服务端确实收到了一条新
    // 连接，这个不会被合并掉。
    home.restart().await;
    phone.dispatch(Action::AppForegrounded).unwrap();
    home.wait_for_a_connection().await;
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.path.is_some())
        .await;
    assert_eq!(
        view.server_id,
        Some(home.server().endpoint().id().to_string())
    );
}

#[tokio::test]
async fn coming_back_after_the_server_dropped_the_connection_takes_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.stop_periodic_checks();
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let proxy = view.proxy.unwrap();
    phone.dispatch(Action::AppBackgrounded).unwrap();

    // App 挂起期间服务端放弃了连接，关闭帧到不了手机：手机仍以为连着，iroh 要 30 秒空闲超时才发现。
    home.drop_connections_silently().await;
    assert_eq!(phone.home().status, HomeStatus::Connected);

    let started = Instant::now();
    phone.dispatch(Action::AppForegrounded).unwrap();
    home.wait_for_a_connection().await;
    let (status, body) = get(&proxy, WEB_URL, "/after", true).await;
    assert_eq!((status, body.as_str()), (200, "/after"));
    assert!(
        started.elapsed() < BEFORE_IDLE_TIMEOUT,
        "回到前台后 {:?} 才恢复",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_dropped_connection_is_replaced_without_coming_back_to_the_foreground() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    // 手机一无所知，也没有回前台、没有发请求：定时检查发现连接不通，换上新连接。
    home.drop_connections_silently().await;
    home.wait_for_a_connection().await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.path.is_some())
        .await;
}

#[tokio::test]
async fn a_network_change_replaces_a_dead_connection_without_waiting_for_the_next_check() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.stop_periodic_checks();
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    home.drop_connections_silently().await;
    let started = Instant::now();
    phone
        .dispatch(Action::NetworkChanged {
            network: DeviceNetwork::Wifi,
        })
        .unwrap();
    home.wait_for_a_connection().await;

    assert!(
        started.elapsed() < BEFORE_IDLE_TIMEOUT,
        "网络变化后 {:?} 才换上新连接",
        started.elapsed()
    );
}

/// 保持连接（ADR-0003）：App 在后台、方式是保持连接时，核心不算在后台——探测和自动重连照常，
/// 断掉的连接不回前台也换新（对照 [`in_the_background_the_core_leaves_the_connection_alone`]）。
#[tokio::test]
async fn keep_connection_replaces_a_dropped_connection_while_backgrounded() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    // 已配对、方式是保持连接：keep_connection 变真，退到后台核心也照样保活。
    phone
        .dispatch(Action::SetPushMethod {
            method: PushMethod::Keepalive,
        })
        .unwrap();
    phone.dispatch(Action::AppBackgrounded).unwrap();
    phone.wait_for(|view| view.keep_connection).await;

    home.drop_connections_silently().await;
    home.wait_for_a_connection().await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.path.is_some())
        .await;
}

#[tokio::test]
async fn in_the_background_the_core_leaves_the_connection_alone() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    // 后台暂停的前提是收新消息走 UnifiedPush（ADR-0003）：保持连接是默认方式，退到后台也照常探测和重连。
    phone
        .dispatch(Action::SetPushMethod {
            method: PushMethod::UnifiedPush,
        })
        .unwrap();
    phone.wait_for(|view| !view.keep_connection).await;
    phone.dispatch(Action::AppBackgrounded).unwrap();
    home.drop_connections_silently().await;
    // 比定时检查的间隔加限时都长。
    tokio::time::sleep(Duration::from_secs(14)).await;
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    assert_eq!(opened.get(), 0, "在后台还在重连");

    phone.dispatch(Action::AppForegrounded).unwrap();
    home.wait_for_a_connection().await;
}

#[tokio::test]
async fn background_requests_keep_the_connection_when_backgrounded_before_connecting() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.dispatch(Action::AppBackgrounded).unwrap();
    phone.connect(&home);
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let proxy = view.proxy.unwrap();
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    let connections = opened.get();

    // resume 还在执行时再次进后台，不能被旧的前台任务断流或覆盖最终后台状态。
    phone.dispatch(Action::AppForegrounded).unwrap();
    phone.dispatch(Action::AppBackgrounded).unwrap();
    // 进程没有被挂起，正常计时超过恢复阈值也不应换连接，后台请求仍可继续使用原代理。
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let (status, body) = get(&proxy, WEB_URL, "/background", true).await;
        assert_eq!((status, body.as_str()), (200, "/background"));
    }
    assert_eq!(opened.get(), connections, "后台正常请求不应换连接");

    phone.dispatch(Action::AppForegrounded).unwrap();
    let (status, body) = get(&proxy, WEB_URL, "/foreground", true).await;
    assert_eq!((status, body.as_str()), (200, "/foreground"));
    assert_eq!(opened.get(), connections, "健康连接回前台不应被替换");
}

/// 抖一下没有提示；断得久一点说“连接中”；回来了自动消失。说得久的情况在 `home::connection` 的测试里。
#[tokio::test]
async fn an_outage_comes_and_goes_in_plain_words() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let connected = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_eq!(connected.notice, None);

    let port = home.stop().await;
    let view = phone
        .wait_for(|view| view.notice == Some(ConnectionNotice::Connecting))
        .await;
    assert!(view.details.disconnects >= 1, "{view:?}");
    assert!(view.details.last_connected_at_ms.is_some(), "{view:?}");

    home.start_again(port).await;
    phone.dispatch(Action::RetryConnection).unwrap();
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_eq!(view.notice, None);
}

#[tokio::test]
async fn retrying_reaches_a_device_paired_after_it_was_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.notice == Some(ConnectionNotice::DeviceRejected))
        .await;

    home.allowlist.allow(phone.device_id());
    phone.dispatch(Action::RetryConnection).unwrap();

    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_eq!(view.notice, None);
}
