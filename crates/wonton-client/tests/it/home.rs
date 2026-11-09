//! 和家里服务器连接的行为测试：只经由导出给原生壳的接口（`ClientCore`、`Action`、订阅）驱动。
//! 隧道服务端、插件上游都跑在本机，只走回环地址，不依赖外网。

mod migration;
pub(crate) mod support;

pub(crate) use support::{Home, Phone};
use support::{WEB_URL, get, wait_until_ticket_has};

use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::{Duration, Instant};

use iroh::EndpointId;
use wonton_client::{
    Action, ConnectionNotice, CoreError, DeviceNetwork, HomeFailure, HomeStatus, HomeViewModel,
    PushMethod,
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
                // 机器忙的时候，网络通知后的确认（4 秒）会超时，连接被换掉：换连接自己会推几条，和这里数的
                // “同一网络没变化也重算”无关，只数仍停在“已连接”的推送。
                publications += u64::from(view.status == HomeStatus::Connected);
            }
        }
    }
    let elapsed = started.elapsed();
    let limit = elapsed.as_secs() + 3;
    eprintln!("300 requests: {publications} home publications in {elapsed:?}, limit {limit}");
    assert!(publications <= limit, "{publications} > {limit}");
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.details.sent_packets > before.details.sent_packets
        })
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
    let moved = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        wonton_tunnel::test_support::unused_port(),
    ));
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
    assert_eq!(status, 200, "{:?}", phone.home());
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
    // 旧连接被判死时断开计数加一（换连接先写“连接中”，同样算一次断开）。提前记下来：关闭帧偶尔能送到，
    // 这一步就会涨；之后涨得比它多只可能来自前台检查对旧连接的判死。
    let disconnects = phone.home().details.disconnects;
    phone.dispatch(Action::AppBackgrounded).unwrap();

    // App 挂起期间服务端放弃了连接，关闭帧到不了手机：手机仍以为连着，iroh 要 30 秒空闲超时才发现。
    home.drop_connections_silently().await;
    assert_eq!(phone.home().status, HomeStatus::Connected);

    // 回到前台：限时确认发现连接已死，马上换新连接。“几秒内回来”不按墙钟断言（机器忙时真实 I/O 会排过任何
    // 时限），按因果断言：定时检查已停、App 刚回前台，先等旧连接被判死（断开计数涨），再等界面重新说“已连接”
    // ——那之后只可能是前台检查换的新连接：空闲超时只会判死、不会自己拨号，“重试”也不动活着的连接。等旧连接
    // 判死之前，界面的“已连接”可能是旧连接的残影。
    phone.dispatch(Action::AppForegrounded).unwrap();
    phone
        .wait_for(|view| view.details.disconnects > disconnects)
        .await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.path.is_some())
        .await;
    let (status, body) = get(&proxy, WEB_URL, "/after", true).await;
    assert_eq!((status, body.as_str()), (200, "/after"));
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

    // 手机一无所知，也没有回前台、没有发请求：网络变化的通知确认连接不通，换上新连接。“不等下一次检查”不按
    // 墙钟断言（机器忙时真实 I/O 会排过任何时限），按因果断言：定时检查已停，服务端看到的新连接只可能是网络
    // 变化这条路径换的——空闲超时只会判死、不会自己拨号。
    home.drop_connections_silently().await;
    phone
        .dispatch(Action::NetworkChanged {
            network: DeviceNetwork::Wifi,
        })
        .unwrap();
    home.wait_for_a_connection().await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.path.is_some())
        .await;
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
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let proxy = view.proxy.unwrap();

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
    let opened = home
        .server()
        .endpoint()
        .metrics()
        .socket
        .num_conns_opened
        .get();
    // 退到后台那一刻已经开出去的那次检查不算：它会把连接换掉、在“后台”留一条。还在后台检查的话，14 秒里
    // （间隔 5 秒、每次最多 8 秒）不止一条。
    assert!(opened <= 1, "在后台还在重连：{opened}");

    // 回到前台，等这一轮检查走完再发请求：它还活着的连接会确认一次，已经判死的会换新的再确认。请求早于
    // 这一轮发出的话，可能正赶上旧连接被收走（502），那不是这个测试要断言的事。
    let confirmed = phone.home().details.last_connected_at_ms;
    phone.dispatch(Action::AppForegrounded).unwrap();
    phone
        .wait_for(|view| view.details.last_connected_at_ms > confirmed)
        .await;
    let (status, body) = get(&proxy, WEB_URL, "/after", true).await;
    assert_eq!((status, body.as_str()), (200, "/after"));
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

/// 抖一下没有提示；第一次重连失败就如实说连不上；回来了自动消失。
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
    let view = phone.wait_for(first_unreachable).await;
    assert!(view.details.disconnects >= 1, "{view:?}");
    assert!(view.details.last_connected_at_ms.is_some(), "{view:?}");

    home.start_again(port).await;
    phone.dispatch(Action::RetryConnection).unwrap();
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_eq!(view.notice, None);
}

/// 提示里“连不上”的那一档，配没配中继都算数。
pub(crate) fn first_unreachable(view: &HomeViewModel) -> bool {
    matches!(
        view.notice,
        Some(ConnectionNotice::Unreachable { .. } | ConnectionNotice::ServerOffline { .. })
    )
}

/// 先失败、再连上、再断开：新一轮还没失败过时，不能把上一轮的失败带出来；“详情”里的上次失败照旧保留。
#[tokio::test]
async fn a_new_outage_does_not_inherit_the_previous_failure() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    // 先失败：这一轮建连失败过，马上如实说连不上。
    let port = home.stop().await;
    let view = phone.wait_for(first_unreachable).await;
    assert!(view.details.last_failure.is_some(), "{view:?}");

    // 再连上：旧失败原文留在详情里（排查信息不因连上丢掉），但状态不带出来。
    home.start_again(port).await;
    phone.dispatch(Action::RetryConnection).unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert!(
        phone.home().details.last_failure.is_some(),
        "{:?}",
        phone.home()
    );

    // 再断开。停掉定时检查后，聊天、设备列表等后台重试仍可能在这一轮重拨；重拨失败，宽限过后提示就会
    // 从“连接中”变“连不上”——那是这一轮自己的失败，证据是失败计数涨了。继承来的失败不会让计数变多。
    phone.stop_periodic_checks();
    let failures_before = phone.home().details.failed_dials;
    home.stop().await;
    let view = phone
        .wait_for(|view| {
            // 30 秒的长断开兜底在等待上限之外：这里说连不上只能来自本轮失败。
            if first_unreachable(view) {
                assert!(
                    view.details.failed_dials > failures_before,
                    "没有本轮失败却说连不上：{view:?}"
                );
                return true;
            }
            view.status != HomeStatus::Connected
                && view.notice == Some(ConnectionNotice::Connecting)
        })
        .await;
    assert!(view.details.last_failure.is_some(), "{view:?}");
}

/// 切到连不上的服务器：第一次建连失败就说连不上，不等 30 秒；自动重试不占着“重试”按钮。
#[tokio::test]
async fn switching_to_an_unreachable_server_says_so_after_the_first_failure() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let mut b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.server_id.as_deref()
                    == Some(a.server().endpoint().id().to_string().as_str())
        })
        .await;
    phone.dispatch(Action::AddServer).unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::NotConfigured)
        .await;
    phone.connect(&b);
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.server_id.as_deref()
                    == Some(b.server().endpoint().id().to_string().as_str())
        })
        .await;
    phone
        .dispatch(Action::SwitchServer { id: "1".into() })
        .unwrap();
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.server_id.as_deref()
                    == Some(a.server().endpoint().id().to_string().as_str())
        })
        .await;

    b.stop().await;
    phone
        .dispatch(Action::SwitchServer { id: "2".into() })
        .unwrap();
    // 旧行为要等满 30 秒才换说法；第一次失败就试（对死端口要等到 12 秒的建连限时，上限留出余量但仍小于 30 秒）。
    let view = tokio::time::timeout(Duration::from_secs(20), phone.wait_for(first_unreachable))
        .await
        .expect("第一次建连失败后就要显示连不上");
    assert!(view.details.last_failure.is_some(), "{view:?}");
    assert!(
        matches!(
            view.notice,
            Some(ConnectionNotice::Unreachable { retrying: false })
                | Some(ConnectionNotice::ServerOffline { retrying: false })
        ),
        "{view:?}"
    );
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
