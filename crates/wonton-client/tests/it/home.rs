//! 和家里服务器连接的行为测试：只经由导出给原生壳的接口（`ClientCore`、`Action`、订阅）驱动。
//! 隧道服务端、插件上游都跑在本机，只走回环地址，不依赖外网。

mod address_refresh;
mod connect;
mod identity;
mod migration;
mod pairing;
mod publication;
mod relay_token;
mod reported_relays;
pub(crate) mod support;

mod quality;
mod rebuild;
mod relays;
mod retry;
mod slow_dial;
mod stuck_bind;
pub(crate) use support::{Home, Phone};
use support::{WEB_URL, get, wait_until_ticket_contains, wait_until_ticket_has};

use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::{Duration, Instant};

use iroh::EndpointId;
use wonton_client::{
    Action, ConnectionNotice, CoreError, DeviceNetwork, HomeFailure, HomeStatus, HomeViewModel,
    PushMethod,
};

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
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
    home.wait_for_a_connection(&phone).await;
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
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();

    // App 挂起期间服务端放弃了连接，关闭帧到不了手机：手机仍以为连着，iroh 要 30 秒空闲超时才发现。后台不是
    // 死水：挂起检测自己也会计时查一次，机器一卡它就先把界面挪成“连接中”，所以这里不断言还停在“已连接”。
    home.drop_connections_silently().await;

    // 回到前台：限时确认发现连接已死，马上换新连接。“几秒内回来”不按墙钟断言（机器忙时真实 I/O 会排过任何
    // 时限），按因果断言：定时检查已停、App 刚回前台，重启后的服务端上出现的新连接只可能是这次换的——空闲
    // 超时只会判死、不会自己拨号，“重试”也不动活着的连接。看服务端自己数出来的连接数，不看 App 侧记的断开
    // 次数：那是 App 的账，换连接的写法一变就可能不涨；等界面说“已连接”且路径在，再发一条请求走通。
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
    home.wait_for_a_connection(&phone).await;
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
    home.wait_for_a_connection(&phone).await;
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
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::NetworkChanged {
                network: DeviceNetwork::Wifi,
            },
        })
        .unwrap();
    home.wait_for_a_connection(&phone).await;
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
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushMethod {
                method: PushMethod::Keepalive,
            },
        })
        .unwrap();
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
    phone.wait_for(|view| view.keep_connection).await;

    home.drop_connections_silently().await;
    home.wait_for_a_connection(&phone).await;
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
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushMethod {
                method: PushMethod::UnifiedPush,
            },
        })
        .unwrap();
    phone.wait_for(|view| !view.keep_connection).await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
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
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
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
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
    phone.connect(&home);
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let proxy = view.proxy.unwrap();
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    let connections = opened.get();

    // resume 还在执行时再次进后台，不能被旧的前台任务断流或覆盖最终后台状态。
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
    // 进程没有被挂起，正常计时超过恢复阈值也不应换连接，后台请求仍可继续使用原代理。
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let (status, body) = get(&proxy, WEB_URL, "/background", true).await;
        assert_eq!((status, body.as_str()), (200, "/background"));
    }
    assert_eq!(opened.get(), connections, "后台正常请求不应换连接");

    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
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
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::RetryConnection,
        })
        .unwrap();
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_eq!(view.notice, None);
}

/// 断开又连上：核心给出“刚恢复”（宽限内的抖动不算），4 秒的窗口由核心收掉，壳不计时。
#[tokio::test]
async fn a_recovered_connection_announces_itself_for_four_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let connected = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    // 一直连着谈不上恢复。
    assert!(!connected.just_recovered, "{connected:?}");

    let port = home.stop().await;
    let down = phone.wait_for(first_unreachable).await;
    assert!(!down.just_recovered, "{down:?}");

    home.start_again(port).await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::RetryConnection,
        })
        .unwrap();
    let recovered = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.just_recovered)
        .await;
    assert_eq!(recovered.notice, None, "{recovered:?}");

    let settled = tokio::time::timeout(
        Duration::from_secs(8),
        phone.wait_for(|view| !view.just_recovered),
    )
    .await
    .expect("恢复的提示到点要由核心收掉");
    assert_eq!(settled.status, HomeStatus::Connected, "{settled:?}");
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
    let summary = view.details.last_failure.as_ref().expect("应有失败说明");
    let diagnostic = view.details.diagnostic.as_ref().expect("应有原始诊断");
    assert!(
        !summary.contains("tunnel transport unavailable"),
        "{summary}"
    );
    assert!(diagnostic.contains("transport-unavailable"), "{diagnostic}");

    // 再连上：上次失败和独立诊断都保留，但状态不带出来。
    home.start_again(port).await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::RetryConnection,
        })
        .unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert!(
        phone.home().details.last_failure.is_some(),
        "{:?}",
        phone.home()
    );
    assert!(
        phone.home().details.diagnostic.is_some(),
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
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AddServer,
        })
        .unwrap();
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
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::SwitchServer { id: "1".into() },
        })
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
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::SwitchServer { id: "2".into() },
        })
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
