//! 前后台切换与换连接：回前台、网络变化、保持连接时自己换掉死连接，真正在后台时不动连接。

use std::time::Duration;

use wonton_client::{Action, DeviceNetwork, HomeStatus, PushMethod};

use super::support::{Home, Phone, WEB_URL, get};

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
