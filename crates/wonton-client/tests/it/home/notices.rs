//! 断连提示的措辞：第一次失败就如实说“连不上”，恢复后给四秒宽限，新一轮不继承上一轮的失败。

use std::time::Duration;

use wonton_client::{Action, ConnectionNotice, HomeStatus, HomeViewModel};

use super::support::{Home, Phone};

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
pub(super) fn first_unreachable(view: &HomeViewModel) -> bool {
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
