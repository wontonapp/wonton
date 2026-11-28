//! 推送唤醒被杀掉的 App：登记订阅，进程取回消息在本机解密并给出通知；分发器注销后清掉家里的登记。

use super::*;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_bot_reply_wakes_a_killed_app_and_shows_the_decrypted_text() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    phone.use_unified_push().await;

    // 地址或公钥不对的订阅，壳给的时候就被拒绝。
    let mut broken = endpoint();
    broken.p256dh = "not-a-key".to_owned();
    assert!(matches!(
        phone.dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint { endpoint: broken }
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    phone
        .wait_chat("推送已登记", |view| {
            (view.push == PushStatus::Registered).then_some(())
        })
        .await;
    // 开机按默认方式（保持连接）会清一次家里的登记，但 unregister 拿着锁等登录，醒来时壳可能已经切到
    // UnifiedPush 并给来了地址：这时直接登记新地址，不再发 DELETE。所以序列是 ["DELETE", "PUT"] 或只有
    // ["PUT"]，都对；PUT 之后再 DELETE 不会出现：清登记和登记共用一把锁，开机那次清登记先拿到、先做完。
    // `register` 在 PUT 返回、注册完 pusher 之后才发布 Registered，看到它时请求已经记全，不用再等。
    let requests = home.push_requests();
    let methods: Vec<_> = requests.iter().map(|(method, _)| method.as_str()).collect();
    assert!(
        matches!(methods.as_slice(), ["PUT"] | ["DELETE", "PUT"]),
        "家里的登记请求序列不对：{methods:?}"
    );
    assert_eq!(
        &requests.last().unwrap().1,
        &json!({"endpoint": ENDPOINT, "keys": {"p256dh": P256DH, "auth": AUTH}}),
        "登记给家里的是 W3C 推送订阅的格式"
    );

    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;

    // App 被杀，Bot 这时回复。
    drop(phone);
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    let sent = bot_room.send_text("你的快递到了", &[]).await;
    sent.expect("Bot 回复");
    let pushed = home
        .push
        .wait_for("推送代理收到 Bot 回复的通知", |received| {
            received.notification["type"] == "m.room.message"
                && received.notification["devices"][0]["pushkey"] == home.pushkey()
        })
        .await;

    // 推送唤醒的进程：用同一个数据目录创建核心，马上把推送交给它。
    let woken = Phone::open(dir.path());
    let notification = woken
        .resolve_push(device_payload(&home, &pushed))
        .await
        .expect("推送内容是对的")
        .expect("这条消息要通知");
    assert_eq!(notification.room_id, room_id.as_str());
    assert_eq!(notification.sender.as_deref(), Some("Hermes"));
    assert_eq!(notification.room_name, None, "私聊，标题用发送者");
    assert_eq!(
        notification.content,
        PushContent::Message {
            summary: MessageSummary::Text {
                text: "你的快递到了".to_owned()
            }
        }
    );
    assert!(matches!(
        woken.resolve_push(b"not json".to_vec()).await,
        Err(CoreError::InvalidInput { .. })
    ));

    // 分发器注销了本 App：照实显示，家里的登记和 pusher 都删掉。
    woken.signed_in().await;
    woken.use_unified_push().await;
    woken
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    woken
        .wait_chat("重新登记", |view| {
            (view.push == PushStatus::Registered).then_some(())
        })
        .await;
    woken
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::PushUnavailable {
                message: "分发器注销了本 App".to_owned(),
            },
        })
        .unwrap();
    assert_eq!(
        woken.chat().push,
        PushStatus::Unavailable {
            message: "分发器注销了本 App".to_owned()
        }
    );
    wait_for_unregistered(&home).await;
}

async fn wait_for_unregistered(home: &Home) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while !home
        .push_requests()
        .iter()
        .any(|(method, _)| method == "DELETE")
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "家里的推送登记没被取消：{:?}",
            home.push_requests()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
