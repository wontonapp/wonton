//! 插件的 Matrix widget 页（ADR-0096）：官方 matrix-widget-api 当页面，经核心的桥和 matrix-sdk 的 widget 驱动
//! 跟真实的 Tuwunel 打交道。能力只批页面请求的 ∩ 清单声明的，发状态事件只能用自己的 `state_key`；
//! 房间的视图列出 widget 入口，别的设备加入通话后房间和聊天列表的这一行显示“通话中”。

mod page;

use std::time::Duration;

use serde_json::{Value, json};
use wonton_client::{
    Action, InstalledPlugin, PagesViewModel, PluginWidget, TabRef, TimelineUpdate, WidgetEntry,
};
use wonton_matrix::OwnedUserId;

use super::accept_the_bots_invite;
use super::support::{Home, OWNER, Phone, Timeline, WAIT};
use page::WidgetPage;

const PLUGIN: &str = "app.call";
const PING: &str = "org.wonton.test.ping";
/// ruma 和 Element Call 用的 `m.call.member` 事件类型（MSC3401 的不稳定写法）。
const CALL_MEMBER: &str = "org.matrix.msc3401.call.member";
const SEND_MEMBER: &str = "org.matrix.msc2762.send.state_event:org.matrix.msc3401.call.member";
const RECEIVE_MEMBER: &str =
    "org.matrix.msc2762.receive.state_event:org.matrix.msc3401.call.member";
const SEND_PING: &str = "org.matrix.msc3819.send.to_device:org.wonton.test.ping";
const RECEIVE_PING: &str = "org.matrix.msc3819.receive.to_device:org.wonton.test.ping";
/// 清单没声明，页面请求了也不批。
const DELAYED: &str = "org.matrix.msc4157.send.delayed_event";

fn plugin() -> InstalledPlugin {
    let mut plugin = crate::plugins::rig::running(PLUGIN);
    plugin.web_url = Some("http://app.call.wonton.localhost/".to_owned());
    plugin.matrix_widget = Some(PluginWidget {
        url: "/room?widgetId=$matrix_widget_id&deviceId=$org.matrix.msc2873.matrix_device_id\
              &parentUrl=$org.wonton.page_origin"
            .to_owned(),
        entry: WidgetEntry::VideoCall,
        capabilities: [SEND_MEMBER, RECEIVE_MEMBER, SEND_PING, RECEIVE_PING]
            .map(str::to_owned)
            .to_vec(),
    });
    plugin
}

/// 起一台服务器、手机登录、Bot 拉主人进加密私聊，打开这个房间的时间线。
async fn in_the_bots_room(
    dir: &std::path::Path,
) -> (
    Home,
    Phone,
    wonton_matrix::MatrixSession,
    String,
    String,
    Timeline,
) {
    let phone = Phone::open(dir);
    let home = Home::start(&[phone.device_id()]).await;
    home.set_installed_plugins(vec![plugin()]);
    phone.connect(&home);
    let (_, device) = phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let room = room_id.to_string();
    let timeline = Timeline::open(&phone, &room).await;
    (home, phone, bot, room, device, timeline)
}

async fn until_update(
    timeline: &mut Timeline,
    what: &str,
    check: impl Fn(&TimelineUpdate) -> bool,
) -> TimelineUpdate {
    let waiting = async {
        loop {
            let update = timeline.next().await;
            if check(&update) {
                return update;
            }
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("{what}：{WAIT:?} 内没有等到"))
}

async fn until_pages<R>(
    phone: &Phone,
    what: &str,
    check: impl Fn(&PagesViewModel) -> Option<R>,
) -> R {
    let subscription = phone.subscribe_pages();
    let waiting = async {
        while let Some(view) = subscription.next().await {
            if let Some(found) = check(&view) {
                return found;
            }
        }
        panic!("页面订阅提前结束");
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("{what}：{WAIT:?} 内没有等到，当前：{:#?}", phone.pages()))
}

fn sorted(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut values: Vec<String> = values.into_iter().collect();
    values.sort();
    values
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_widget_gets_only_granted_capabilities_and_talks_to_the_room() {
    let dir = tempfile::tempdir().unwrap();
    let (_home, phone, bot, room, device, mut timeline) = in_the_bots_room(dir.path()).await;
    let tab = TabRef::Widget {
        plugin_id: PLUGIN.to_owned(),
        room_id: room.clone(),
    };
    let entry = until_update(&mut timeline, "房间列出 widget 入口", |update| {
        !update.widgets.is_empty()
    })
    .await;
    assert_eq!(entry.widgets.len(), 1);
    assert_eq!(entry.widgets[0].tab, tab);
    assert_eq!(entry.widgets[0].entry, WidgetEntry::VideoCall);

    phone
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::OpenPage { tab: tab.clone() },
        })
        .unwrap();
    let url = until_pages(&phone, "widget 页的地址出来", |view| {
        view.stack
            .iter()
            .find(|item| item.tab == tab)
            .and_then(|item| item.page.url.clone())
    })
    .await;

    let own_key = format!("_{OWNER}_{device}_m.call");
    let bots_key = format!("_{}_{}_m.call", bot.user_id(), bot.device_id());
    let own = format!("{SEND_MEMBER}#{own_key}");
    let bots = format!("{SEND_MEMBER}#{bots_key}");
    let requested = [
        own.as_str(),
        bots.as_str(),
        RECEIVE_MEMBER,
        SEND_PING,
        RECEIVE_PING,
        DELAYED,
    ];
    let (mut page, ready) =
        WidgetPage::open(phone.core.clone(), tab.clone(), &url, &requested).await;
    assert_eq!(ready["query"]["deviceId"], device.as_str());
    let origin = url::Url::parse(&url)
        .unwrap()
        .origin()
        .ascii_serialization();
    assert_eq!(
        url::Url::parse(ready["query"]["parentUrl"].as_str().unwrap())
            .unwrap()
            .origin()
            .ascii_serialization(),
        origin
    );
    let approved: Vec<String> = serde_json::from_value(ready["approved"].clone()).unwrap();
    assert_eq!(
        sorted(approved),
        sorted([
            own.clone(),
            RECEIVE_MEMBER.to_owned(),
            SEND_PING.to_owned(),
            RECEIVE_PING.to_owned()
        ])
    );

    let member = json!({
        "application": "m.call", "call_id": "", "scope": "m.room", "device_id": device,
        "focus_active": {"type": "livekit", "focus_selection": "oldest_membership"},
        "foci_preferred": [{"type": "livekit", "livekit_service_url": "https://livekit.invalid", "livekit_alias": room}],
    });
    let sent = page
        .call(
            "sendState",
            json!({"type": CALL_MEMBER, "stateKey": own_key, "content": member}),
        )
        .await;
    assert!(sent["result"]["event_id"].is_string(), "{sent}");
    let refused = page
        .call(
            "sendState",
            json!({"type": CALL_MEMBER, "stateKey": bots_key, "content": member}),
        )
        .await;
    assert!(refused["error"].is_string(), "{refused}");

    let openid = page.call("openId", Value::Null).await;
    assert!(
        openid["result"]["access_token"]
            .as_str()
            .is_some_and(|token| !token.is_empty()),
        "{openid}"
    );

    widget_to_bot(&mut page, &bot).await;
    bot_to_widget(&mut page, &bot, &device).await;

    phone
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::ClosePage,
        })
        .unwrap();
    page.ended().await;
}

/// 页面经驱动发加密的 to-device 给 Bot 的设备。手机可能还没拿到 Bot 的设备密钥，送不到就再发。
async fn widget_to_bot(page: &mut WidgetPage, bot: &wonton_matrix::MatrixSession) {
    let mut received = bot.to_device_events(PING);
    for attempt in 0.. {
        assert!(attempt < 10, "Bot 一直没收到 widget 发的 to-device");
        let messages =
            json!({bot.user_id().as_str(): {bot.device_id().as_str(): {"attempt": attempt}}});
        let sent = page
            .call(
                "sendToDevice",
                json!({"type": PING, "encrypted": true, "messages": messages}),
            )
            .await;
        assert!(sent.get("error").is_none(), "{sent}");
        if let Ok(Some((content, encrypted))) =
            tokio::time::timeout(Duration::from_secs(10), received.recv()).await
        {
            assert!(encrypted, "加密房间里的 to-device 要加密送");
            assert!(content["attempt"].is_number(), "{content}");
            return;
        }
    }
}

/// Bot 发给手机这台设备的加密 to-device，驱动交给页面，标着加密。
async fn bot_to_widget(page: &mut WidgetPage, bot: &wonton_matrix::MatrixSession, device: &str) {
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let device: wonton_matrix::OwnedDeviceId = device.into();
    for attempt in 0.. {
        assert!(attempt < 20, "widget 一直没收到 Bot 发的 to-device");
        let delivered = bot
            .send_encrypted_to_device(&owner, &device, PING, &json!({"hello": "widget"}))
            .await
            .expect("Bot 发 to-device");
        if delivered {
            for _ in 0..20 {
                let received = page.call("received", Value::Null).await;
                if let Some(event) = received["result"]
                    .as_array()
                    .and_then(|events| events.first())
                {
                    assert_eq!(event["type"], PING);
                    assert_eq!(event["sender"], bot.user_id().as_str());
                    assert_eq!(event["content"]["hello"], "widget");
                    assert_eq!(event["encrypted"], true);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_room_shows_a_call_once_another_device_joins() {
    let dir = tempfile::tempdir().unwrap();
    let (_home, phone, bot, room, _device, mut timeline) = in_the_bots_room(dir.path()).await;
    let quiet = until_update(&mut timeline, "房间列出 widget 入口", |update| {
        !update.widgets.is_empty()
    })
    .await;
    assert!(!quiet.call_active);
    let row_in_call = |view: &wonton_client::ChatViewModel| {
        view.rooms
            .iter()
            .find(|row| row.room_id == room)
            .map(|row| row.call_active)
    };
    assert_eq!(row_in_call(&phone.chat()), Some(false));

    bot.join_call(&room.parse::<wonton_matrix::OwnedRoomId>().unwrap())
        .await
        .expect("Bot 加入通话");
    let ringing = until_update(&mut timeline, "房间显示通话中", |update| {
        update.call_active
    })
    .await;
    assert_eq!(ringing.widgets.len(), 1);
    // 聊天列表的这一行跟着标“通话中”。
    phone
        .wait_chat("列表这一行显示通话中", |view| {
            row_in_call(view).filter(|active| *active)
        })
        .await;
}
