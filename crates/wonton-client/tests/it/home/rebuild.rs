//! 本机 endpoint 死掉时整个重建，家里服务器死了不重建（ADR-0032 §4）。
//!
//! 检查失败本身不当证据：服务器死掉、断电时检查也失败，网络却好好的。重建要靠"连接信息里有中继表，
//! 而一个中继都不在连、设备网络又在位"——中继连接走和直连同一个 UDP socket，socket 死了中继跟着掉线；
//! 服务器死掉中继还在，这组证据分得清是本机坏了还是家里不通。iroh 把本机 socket 上的发送错误整个吞掉
//! （发送失败只按丢包重传，拨号只会超时），endpoint 关闭后中继状态整个清空，没有更直接的证据可查。
//! 测试里用 iroh 的回环测试中继当中继表：(a) 服务器死掉、中继还在，不重建；(b) 用 `test_close_home_endpoint`
//! 关掉 endpoint，中继跟着断，重建。

use std::time::{Duration, Instant};

use iroh::test_utils::run_relay_server;
use wonton_client::{Action, DeviceNetwork, HomeStatus, HomeViewModel, Network};

use super::support::{Home, Phone, WEB_URL, get};

#[tokio::test]
async fn a_dead_server_never_rebuilds_the_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    // 连接信息带上回环测试中继：服务器死了检查会连着失败，但中继还在线——这正是重建证据要排除的情形
    // （旧规则"检查连续失败 2 次就重建"在这里会误重建，本测试在旧规则下是红的；见 PR 正文）。
    let phone = Phone::with_network(dir.path(), Network::TestRelay);
    let mut home = Home::start(&[phone.device_id()]).await;
    let (_relay_map, relay_url, _relay) = run_relay_server().await.unwrap();
    phone
        .dispatch(Action::ConnectHome {
            ticket: ticket_with_relay(&home, &relay_url),
        })
        .unwrap();
    phone
        .dispatch(Action::NetworkChanged {
            network: DeviceNetwork::Wifi,
        })
        .unwrap();
    let first = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let password = first.proxy.unwrap().password;

    // 服务器没了，中继还在、网络在位：观察 95 秒以上，凡是流过的视图，本地代理的凭据一个都不能变。
    // 稳态断开时视图只在变化时投递，退避重拨的间隙里会静默，不能要求它持续流动。
    home.stop().await;
    let deadline = Instant::now() + Duration::from_secs(95);
    let subscription = phone.subscribe_home();
    while Instant::now() < deadline {
        let quiet = Duration::from_secs(60);
        match tokio::time::timeout(quiet, subscription.next()).await {
            Ok(Some(view)) => assert_eq!(
                view.proxy.as_ref().map(|p| p.password.as_str()),
                Some(password.as_str()),
                "服务器死了不该重建 endpoint"
            ),
            Ok(None) => panic!("订阅提前结束"),
            Err(_) => {} // 静默期：退避重拨之间视图没有变化，继续等到总期限。
        }
    }
}

#[tokio::test]
async fn a_dead_local_endpoint_is_rebuilt_when_the_relays_are_gone_and_the_network_holds() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::with_network(dir.path(), Network::TestRelay);
    let home = Home::start(&[phone.device_id()]).await;
    let (_relay_map, relay_url, _relay) = run_relay_server().await.unwrap();
    phone
        .dispatch(Action::ConnectHome {
            ticket: ticket_with_relay(&home, &relay_url),
        })
        .unwrap();
    phone
        .dispatch(Action::NetworkChanged {
            network: DeviceNetwork::Wifi,
        })
        .unwrap();
    let first = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let password = first.proxy.unwrap().password;

    // 关掉 endpoint：这条 socket 上的连接和中继一起失效。连续两轮"检查失败且一个中继都不在"就重建，
    // 服务器一直活着，重建后的 endpoint 自己连上，代理能用。等"换了凭据且已连上"一起出现：凭据先于
    // 建连上视图，只等凭据会在 Connecting 时返回，打在还没接通的隧道上。
    phone.test_close_home_endpoint();
    let rebuilt = wait_for_longer(&phone, |view| {
        view.status == HomeStatus::Connected
            && view.proxy.as_ref().is_some_and(|p| p.password != password)
    })
    .await;
    // 断开次数按这台服务器算：旧 endpoint 上那次断开不因为换了新的隧道客户端清零。
    assert!(rebuilt.details.disconnects >= 1, "{:?}", rebuilt.details);
    let second = rebuilt
        .proxy
        .as_ref()
        .expect("重建后代理要回来")
        .password
        .clone();
    let (status, body) = get(
        &rebuilt.proxy.expect("重建后代理要回来"),
        WEB_URL,
        "/hello",
        true,
    )
    .await;
    assert_eq!((status, body.as_str()), (200, "/hello"));

    // 冷却期内（30 秒）endpoint 又死了：spawn 被冷却挡下，检查循环得活着继续试，冷却一过第二次重建
    // 照样发生，凭据再次轮换。
    phone.test_close_home_endpoint();
    let again = wait_for_longer(&phone, |view| {
        view.status == HomeStatus::Connected
            && view.proxy.as_ref().is_some_and(|p| p.password != second)
    })
    .await;
    let (status, body) = get(
        &again.proxy.expect("第二次重建后代理要回来"),
        WEB_URL,
        "/hello",
        true,
    )
    .await;
    assert_eq!((status, body.as_str()), (200, "/hello"));
}

/// 连接信息里加上回环测试中继：手机的中继表来自它，重建的证据才有得看。
fn ticket_with_relay(home: &Home, relay_url: &iroh::RelayUrl) -> String {
    let id = home.server().endpoint().id();
    let addr = home.server().endpoint().bound_sockets()[0];
    format!(
        r#"{{"server": {{"endpoint_id": "{id}", "relays": [{{"url": "{relay_url}", "tier": 0}}],
            "direct_addrs": ["{addr}"]}},
            "base_domain": "wonton.localhost", "web_url": "{WEB_URL}"}}"#
    )
}

/// 和 `Phone::wait_for` 一样等视图，上限放宽：重建要等连续两轮失败的检查。
async fn wait_for_longer(
    phone: &Phone,
    mut done: impl FnMut(&HomeViewModel) -> bool,
) -> HomeViewModel {
    let subscription = phone.subscribe_home();
    let waiting = async {
        while let Some(view) = subscription.next().await {
            if done(&view) {
                return view;
            }
        }
        panic!("订阅提前结束");
    };
    tokio::time::timeout(Duration::from_secs(90), waiting)
        .await
        .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", phone.home()))
}
