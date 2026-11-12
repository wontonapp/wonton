//! 收新消息的方式（ADR-0003 的“设计：保持连接”）：存在本机、默认保持连接；已配对加保持连接时
//! `keep_connection` 为真，壳照它启停前台服务；App 在后台时同步到的该通知的消息收集进
//! `background_notifications`，回到前台清空。

use wonton_client::{
    Action, ChatFailure, ChatViewModel, CoreError, DeviceNetwork, MessageSummary, PushContent,
    PushMethod, PushStatus,
};
use wonton_matrix::ChatRoom;
use wonton_protocol::SettingsEntry;
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::accept_the_bots_invite;
use super::push::endpoint;
use super::support::{Home, Phone, ToDeviceHold, WAIT};

/// 自己带出去的那条会同步回来，但不该进后台通知；同一条别的照常收集。
async fn own_message_is_not_notified(phone: &Phone, bot_room: &ChatRoom, room_id: &str) {
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room_id.to_owned(),
                    thread_root: None,
                },
                body: "这是我自己说的".to_owned(),
                reply_to: None,
            },
        })
        .unwrap();
    bot_room.send_text("收到").await.expect("Bot 再发一条");
    let mine = phone
        .wait_chat("第二条该通知的消息", |view| {
            view.background_notifications
                .iter()
                .find(|notice| {
                    notice.content
                        == PushContent::Message {
                            summary: MessageSummary::Text {
                                text: "收到".to_owned(),
                            },
                        }
                })
                .map(|notice| notice.sender.clone())
        })
        .await;
    assert_eq!(mine.as_deref(), Some("Hermes"));
    assert!(
        !phone
            .chat()
            .background_notifications
            .iter()
            .any(|notice| notice.content
                == PushContent::Message {
                    summary: MessageSummary::Text {
                        text: "这是我自己说的".to_owned()
                    }
                }),
        "自己发的消息不该通知"
    );
}

/// 这条消息进了房间预览：只有经同步才会出现。
fn preview_has<'a>(room_id: &'a str, text: &'a str) -> impl Fn(&ChatViewModel) -> Option<()> + 'a {
    move |view| {
        let body = MessageSummary::Text {
            text: text.to_owned(),
        };
        view.rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .and_then(|room| room.preview.as_ref())
            .filter(|preview| preview.body == body)
            .map(|_| ())
    }
}

/// 设置页“通知”条目里存的方式，同壳读它的路径。
fn push_method(settings: &wonton_client::SettingsViewModel) -> PushMethod {
    settings
        .sections
        .iter()
        .flat_map(|section| section.entries.iter())
        .find_map(|entry| match entry {
            SettingsEntry::Notifications { method, .. } => Some(*method),
            _ => None,
        })
        .expect("设置里没有通知条目")
}

/// 等设置页的视图满足条件。方式存进文件是同步的，设置页的显示由后台任务跟着重算，要等一个变化。
async fn wait_settings<R>(
    phone: &Phone,
    what: &str,
    check: impl Fn(&wonton_client::SettingsViewModel) -> Option<R>,
) -> R {
    let subscription = phone.subscribe_settings();
    let waiting = async {
        loop {
            let view = subscription.next().await.unwrap();
            if let Some(found) = check(&view) {
                return found;
            }
        }
    };
    match tokio::time::timeout(WAIT, waiting).await {
        Ok(found) => found,
        Err(_) => panic!("{what}：{WAIT:?} 内没有等到，当前：{:#?}", phone.settings()),
    }
}

/// 等家里连接的视图满足条件。`keep_connection` 由后台任务跟着连接状态和方式重算，要等一个变化。
async fn wait_home<R>(
    phone: &Phone,
    what: &str,
    check: impl Fn(&wonton_client::HomeViewModel) -> Option<R>,
) -> R {
    let subscription = phone.subscribe_home();
    let waiting = async {
        loop {
            let view = subscription.next().await.unwrap();
            if let Some(found) = check(&view) {
                return found;
            }
        }
    };
    match tokio::time::timeout(WAIT, waiting).await {
        Ok(found) => found,
        Err(_) => panic!("{what}：{WAIT:?} 内没有等到，当前：{:#?}", phone.home()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_default_is_keepalive_and_rejects_push_endpoints() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    assert_eq!(push_method(&phone.settings()), PushMethod::Keepalive);
    assert!(!phone.home().keep_connection, "还没配对，连接不用保");
    assert!(matches!(
        phone.dispatch(Action::SetPushEndpoint {
            endpoint: endpoint()
        }),
        Err(CoreError::InvalidInput { .. })
    ));

    // 换成 UnifiedPush 后地址就收了（连接还没建，登记留着等会话）。方式存文件是同步的，设置页和推送状态
    // 由后台任务跟着重算，都等它们变。
    phone
        .dispatch(Action::SetPushMethod {
            method: PushMethod::UnifiedPush,
        })
        .unwrap();
    wait_settings(&phone, "设置页换成 UnifiedPush", |view| {
        (push_method(view) == PushMethod::UnifiedPush).then_some(())
    })
    .await;
    assert!(!phone.home().keep_connection);
    phone
        .dispatch(Action::SetPushEndpoint {
            endpoint: endpoint(),
        })
        .unwrap();
    phone
        .wait_chat("推送开始登记", |view| {
            (view.push == PushStatus::Registering).then_some(())
        })
        .await;

    // 换回保持连接：家里的登记和 pusher 删掉（这个测试没连上，删除等会话就绪时尽力而为），状态回到没登记。
    phone
        .dispatch(Action::SetPushMethod {
            method: PushMethod::Keepalive,
        })
        .unwrap();
    wait_settings(&phone, "设置页换回保持连接", |view| {
        (push_method(view) == PushMethod::Keepalive).then_some(())
    })
    .await;
    phone
        .wait_chat("状态回到没登记", |view| {
            (view.push == PushStatus::NoEndpoint).then_some(())
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chosen_method_survives_a_reopened_core() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone
        .dispatch(Action::SetPushMethod {
            method: PushMethod::UnifiedPush,
        })
        .unwrap();
    drop(phone);

    let reopened = Phone::open(dir.path());
    assert_eq!(push_method(&reopened.settings()), PushMethod::UnifiedPush);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn keep_connection_follows_pairing_and_the_method() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    assert!(!phone.home().keep_connection, "还没配对");

    phone.connect(&home);
    phone.signed_in().await;
    // 默认就是保持连接：配上以后连接要保住，壳据此开前台服务。
    wait_home(&phone, "配对后 keep_connection", |view| {
        view.keep_connection.then_some(())
    })
    .await;

    phone
        .dispatch(Action::SetPushMethod {
            method: PushMethod::UnifiedPush,
        })
        .unwrap();
    wait_home(&phone, "换成 UnifiedPush 后不保连接", |view| {
        (!view.keep_connection).then_some(())
    })
    .await;

    phone
        .dispatch(Action::SetPushMethod {
            method: PushMethod::Keepalive,
        })
        .unwrap();
    wait_home(&phone, "换回保持连接", |view| {
        view.keep_connection.then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn background_messages_are_collected_and_cleared_on_foreground() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;

    phone.dispatch(Action::AppBackgrounded).unwrap();
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room.send_text("在吗").await.expect("Bot 发消息");
    // 先等这条从同步到了手机上（房间预览里出现），这里只可能由那次后台同步写进来；通知要在这个基础上再取一次
    // 消息才生成，两件事分开等，通知那一步迟迟不来时不会再被同步本身的耗时吃掉预算。
    phone
        .wait_chat("后台同步到这条消息", preview_has(room_id.as_str(), "在吗"))
        .await;
    let notice = phone
        .wait_chat("后台同步到的通知", |view| {
            view.background_notifications
                .iter()
                .find(|notice| notice.room_id == room_id.as_str())
                .map(|notice| (notice.sender.clone(), notice.content.clone()))
        })
        .await;
    assert_eq!(notice.0.as_deref(), Some("Hermes"));
    assert_eq!(
        notice.1,
        PushContent::Message {
            summary: MessageSummary::Text {
                text: "在吗".to_owned()
            }
        }
    );

    // 自己发的不通知：同步还开着，消息能到本机，但不该进后台通知。
    own_message_is_not_notified(&phone, &bot_room, room_id.as_str()).await;

    // 回到前台清空：主人看得见界面，通知都用不上了。
    phone.dispatch(Action::AppForegrounded).unwrap();
    assert!(phone.chat().background_notifications.is_empty());

    // 后台的网络变化不是回到前台：不清空、继续收集。
    phone.dispatch(Action::AppBackgrounded).unwrap();
    bot_room.send_text("还在").await.expect("Bot 再发一条");
    phone
        .wait_chat("重新开始收集", |view| {
            view.background_notifications
                .iter()
                .find(|notice| {
                    notice.content
                        == PushContent::Message {
                            summary: MessageSummary::Text {
                                text: "还在".to_owned(),
                            },
                        }
                })
                .map(|_| ())
        })
        .await;
    phone
        .dispatch(Action::NetworkChanged {
            network: DeviceNetwork::Wifi,
        })
        .unwrap();
    assert!(
        !phone.chat().background_notifications.is_empty(),
        "后台的网络变化不该清空后台通知"
    );
}

/// 房间密钥晚于等待上限才到时（同步被压住时实测晚 6.6–9.1 秒），占位先进后台通知；密钥到了核心回填，
/// 同一条通知换成正文，不追加第二条。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_late_room_key_replaces_the_unavailable_background_notification() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    // Bot 经扣下 to-device 的中转登录：发消息时房间密钥被扣下（回 200 让它当已送达），消息事件照发，
    // 手机只收到解不开的消息。重放之后密钥才到，和密钥晚到是同一件事。
    let hold = ToDeviceHold::start(&home.matrix.url).await;
    let bot = home.bot_via(hold.url(), "hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;

    phone.dispatch(Action::AppBackgrounded).unwrap();
    hold.hold(true);
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room.send_text("在吗").await.expect("Bot 发消息");

    let event_id = phone
        .wait_chat("解不开的占位先到", |view| {
            view.background_notifications
                .iter()
                .find(|notice| {
                    notice.room_id == room_id.as_str()
                        && matches!(
                            notice.content,
                            PushContent::Unavailable {
                                reason: ChatFailure::NotReady,
                                ..
                            }
                        )
                })
                .map(|notice| notice.event_id.clone())
        })
        .await;

    // 密钥这时才到：核心重取正文、同一条通知再交一次，占位换成正文，也只有这一条。
    hold.replay().await;
    let replaced = phone
        .wait_chat("密钥到了回填成正文", |view| {
            let mut entries = view
                .background_notifications
                .iter()
                .filter(|notice| notice.event_id == event_id)
                .cloned()
                .collect::<Vec<_>>();
            (entries.len() == 1
                && entries[0].content
                    == PushContent::Message {
                        summary: MessageSummary::Text {
                            text: "在吗".to_owned(),
                        },
                    })
            .then(|| entries.remove(0))
        })
        .await;
    assert_eq!(replaced.room_id, room_id.as_str());
    assert_eq!(replaced.sender.as_deref(), Some("Hermes"));
}
