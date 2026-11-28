//! 推送来自别的已配服务器、新地址在每台上都登记、同一个地址只登记一次（ADR-0042）。

use super::*;

/// 推送来自不是当前连着的那台（ADR-0042）：临时连上那台取回、解密，当前这台不受影响。从手机上移除了那台以后，
/// 照实说是已移除的服务器发来的，不显示成消息。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_push_from_a_server_you_are_not_on_is_fetched_from_that_server() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    phone.signed_in().await;
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    let registered =
        |view: &wonton_client::ChatViewModel| (view.push == PushStatus::Registered).then_some(());
    phone.wait_chat("A 上推送已登记", registered).await;
    let bot = a.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&a, &phone, &bot).await;

    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AddServer,
        })
        .unwrap();
    phone.connect(&b);
    let (_, on_b) = phone.signed_in().await;
    phone.wait_chat("B 上推送已登记", registered).await;

    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    let sent = bot_room.send_text("A 上的新消息", &[]).await;
    sent.expect("Bot 回复");
    let pushed = a
        .push
        .wait_for("A 的推送代理收到通知", |received| {
            received.notification["type"] == "m.room.message"
                && received.notification["devices"][0]["pushkey"] == a.pushkey()
        })
        .await;
    let payload = device_payload(&a, &pushed);
    let notification = phone
        .resolve_push(payload.clone())
        .await
        .expect("推送内容是对的")
        .expect("这条消息要通知");
    assert_eq!(notification.server_id.as_deref(), Some("1"));
    assert_eq!(notification.room_id, room_id.as_str());
    assert_eq!(
        notification.content,
        PushContent::Message {
            summary: MessageSummary::Text {
                text: "A 上的新消息".to_owned()
            }
        }
    );
    assert_eq!(phone.signed_in().await.1, on_b, "当前还连着 B");
    assert_eq!(a.login_tokens(), 1, "临时连 A 用的是原来的会话");

    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::RemoveServer { id: "1".into() },
        })
        .unwrap();
    let notification = phone
        .resolve_push(payload)
        .await
        .expect("推送内容是对的")
        .expect("要告诉主人有推送");
    assert_eq!(notification.server_id, None);
    assert!(
        matches!(
            notification.content,
            PushContent::Unavailable {
                reason: ChatFailure::NotReady,
                ..
            }
        ),
        "{:?}",
        notification.content
    );
}

/// 推送地址变了（ADR-0042）：当前这台马上重新登记，配过的另一台在后台临时连上去登记。地址没变时（每次启动壳都会
/// 再给一遍）另一台不用再登记。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_new_push_endpoint_is_registered_on_the_other_servers_too() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    let registered =
        |view: &wonton_client::ChatViewModel| (view.push == PushStatus::Registered).then_some(());
    phone.connect(&a);
    phone.signed_in().await;
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    phone.wait_chat("A 上推送已登记", registered).await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AddServer,
        })
        .unwrap();
    phone.connect(&b);
    phone.signed_in().await;
    phone.wait_chat("B 上推送已登记", registered).await;

    let mut moved = endpoint();
    moved.url = "https://ntfy.example/upWonton456?up=1".to_owned();
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: moved.clone(),
            },
        })
        .unwrap();
    phone.wait_chat("B 上用新地址登记", registered).await;
    let registered_on_a =
        async {
            while !a.push_requests().iter().any(|(method, subscription)| {
                method == "PUT" && subscription["endpoint"] == moved.url
            }) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
    tokio::time::timeout(Duration::from_secs(90), registered_on_a)
        .await
        .expect("A 应在后台用新地址重新登记");
    assert!(
        matches!(phone.home().status, HomeStatus::Connected) && phone.servers().servers[1].current,
        "当前还连着 B"
    );

    let on_a = a.push_requests().len();
    let on_b = b.push_requests().len();
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint { endpoint: moved },
        })
        .unwrap();
    phone.wait_chat("B 再登记一遍", registered).await;
    assert!(b.push_requests().len() > on_b);
    assert_eq!(a.push_requests().len(), on_a, "地址没变，A 不用再登记");
}

/// 等家里到达一条满足条件的推送登记请求；靠假核心的到达计数等事件，不睡固定间隔。
async fn wait_for_a_registration(
    home: &Home,
    within: Duration,
    mut found: impl FnMut(&[(String, Value)]) -> bool,
) -> bool {
    let mut seen = home.push_registrations_seen();
    let waiting = async {
        loop {
            if found(&home.push_requests()) {
                return true;
            }
            if seen.changed().await.is_err() {
                panic!("推送登记的假核心没了");
            }
        }
    };
    tokio::time::timeout(within, waiting).await.unwrap_or(false)
}

/// 另一台还在登记（家里已收到 PUT、登记还没走完）时，壳再给一遍同一个地址（Android 的 UnifiedPush 开机时常常
/// 连着回调几次 `onNewEndpoint`），家里只该收到一次 PUT。登记走完之前只有落盘记录能去重，而记录要等登记走完
/// 才写，所以核心得记着“正在给哪台登记哪个地址”。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_repeated_endpoint_while_another_server_is_still_registering_is_not_registered_again() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    let registered =
        |view: &wonton_client::ChatViewModel| (view.push == PushStatus::Registered).then_some(());
    phone.connect(&a);
    phone.signed_in().await;
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    phone.wait_chat("A 上推送已登记", registered).await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AddServer,
        })
        .unwrap();
    phone.connect(&b);
    phone.signed_in().await;
    phone.wait_chat("B 上推送已登记", registered).await;

    // 挂起 A 的登记响应：家里已收到 PUT、登记停在等响应，正好是写盘前的窗口。
    a.hold_push_registrations(true);
    let mut moved = endpoint();
    moved.url = "https://ntfy.example/upWonton456?up=1".to_owned();
    let puts_with_moved = |requests: &[(String, Value)]| {
        requests
            .iter()
            .filter(|(m, s)| m == "PUT" && s["endpoint"] == moved.url)
            .count()
    };
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: moved.clone(),
            },
        })
        .unwrap();
    assert!(
        wait_for_a_registration(&a, WAIT, |requests| puts_with_moved(requests) > 0).await,
        "家里没等到 A 用新地址登记"
    );

    // 窗口里壳再给一遍同一个地址。放行让挂起的登记走完：重复的登记跟第一次共用同一把登记锁，
    // 它的 PUT 只会出现在放行之后，所以断言放在放行后、给第二次 PUT 一个有界的机会再数。
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: moved.clone(),
            },
        })
        .unwrap();
    a.hold_push_registrations(false);
    let _ = wait_for_a_registration(&a, Duration::from_secs(2), |requests| {
        puts_with_moved(requests) > 1
    })
    .await;
    assert_eq!(
        puts_with_moved(&a.push_requests()),
        1,
        "同一个地址家里只该收到一次 PUT"
    );
}
