//! 断开又恢复，App 一直开着：同步在同一个会话上接着跑，打开着的房间收到断开期间的消息，没发出去的发出去（#154、#93）。

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use wonton_client::{Action, ChatSync, HomeStatus, TimelineContent};
use wonton_matrix::{ChatRoom, MatrixSession, MessageBody, OwnedRoomId};

use super::support::{Home, Phone, Timeline, WAIT};
use super::{accept_the_bots_invite, text};

struct Chat {
    phone: Phone,
    home: Home,
    _bot: MatrixSession,
    bot_room: ChatRoom,
    room_id: OwnedRoomId,
    timeline: Timeline,
    login_tokens: usize,
    _dir: tempfile::TempDir,
}

impl Chat {
    async fn open() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::open(dir.path());
        let home = Home::start(&[phone.device_id()]).await;
        phone.connect(&home);
        phone.signed_in().await;
        let bot = home.bot("hermes", "Hermes").await;
        let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
        let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
        let timeline = Timeline::open(&phone, room_id.as_str()).await;
        let login_tokens = home.login_tokens();
        Self {
            phone,
            home,
            _bot: bot,
            bot_room,
            room_id,
            timeline,
            login_tokens,
            _dir: dir,
        }
    }

    /// 断开期间两边各发一条。
    async fn both_send(&self) {
        self.bot_room
            .send_text("断开时 Bot 发的")
            .await
            .expect("Bot 发消息");
        self.phone
            .dispatch(Action::SendText {
                room_id: self.room_id.to_string(),
                body: "断开时主人发的".to_owned(),
            })
            .unwrap();
    }

    /// 恢复以后两条都到了对面，同步在跑，用的还是原来的会话。
    async fn caught_up(&mut self) {
        self.timeline
            .until("打开着的房间收到断开时 Bot 发的", |item| {
                text(item, "断开时 Bot 发的")
            })
            .await;
        self.bot_received("断开时主人发的").await;
        self.phone
            .wait_chat("同步恢复", |view| {
                (view.sync == ChatSync::Running).then_some(())
            })
            .await;
        assert_eq!(
            self.home.login_tokens(),
            self.login_tokens,
            "恢复不应重新登录"
        );
    }

    /// Bot 收到手机发的那条；等不到时把手机这边的情况一起打出来。
    async fn bot_received(&self, body: &str) {
        let waiting = async {
            let mut updates = self.bot_room.message_updates().await;
            while let Some(messages) = updates.next().await {
                if messages
                    .iter()
                    .any(|message| message.body == MessageBody::Text(body.to_owned()))
                {
                    return;
                }
            }
            panic!("Bot 的消息流提前结束");
        };
        if tokio::time::timeout(WAIT, waiting).await.is_err() {
            panic!(
                "Bot 收到{body}：{WAIT:?} 内没有等到；手机看到的时间线：{:#?}，状态：{:#?}",
                self.timeline.items,
                self.phone.chat(),
            );
        }
    }

    async fn sync_failed(&self) {
        self.phone
            .wait_chat("同步因 Tuwunel 挂了停下", |view| {
                matches!(view.sync, ChatSync::Failed { .. }).then_some(())
            })
            .await;
    }
}

/// 同步失败后不用回前台，自己重启。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sync_restarts_by_itself_after_the_server_comes_back() {
    let mut chat = Chat::open().await;
    chat.home.set_matrix_down(true);
    chat.sync_failed().await;
    chat.both_send().await;

    chat.home.set_matrix_down(false);

    chat.caught_up().await;
}

/// 同步失败时回到前台：接着用同一个会话。换会话的话，打开着的房间还订阅着旧会话，再也收不到更新。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn coming_back_to_the_foreground_keeps_the_open_room_alive() {
    let mut chat = Chat::open().await;
    chat.home.set_matrix_down(true);
    chat.sync_failed().await;
    chat.both_send().await;

    chat.home.set_matrix_down(false);
    chat.phone.dispatch(Action::AppForegrounded).unwrap();

    chat.caught_up().await;
}

/// 隧道断开又恢复。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_chat_recovers_when_the_tunnel_comes_back() {
    let mut chat = Chat::open().await;
    chat.home.stop_tunnel().await;
    // 这时 SDK 的同步请求还挂在本地代理上等重连，报错要等几十秒，状态可能还是“正在同步”；这里只等手机知道断了。
    let deadline = Instant::now() + Duration::from_secs(30);
    while chat.phone.home().status == HomeStatus::Connected {
        assert!(Instant::now() < deadline, "隧道断了，手机还说连着");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    chat.both_send().await;

    chat.home.start_tunnel().await;

    chat.caught_up().await;
}

/// 编辑、表情回应、撤回不会在时间线里自己成一项：它们来的时候不能把整条时间线重建（壳会清屏重画，流式回复时就是
/// 每秒一次）。内容照常更新。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn edits_reactions_and_redactions_do_not_rebuild_the_timeline() {
    let mut chat = Chat::open().await;
    chat.bot_room
        .send_text("流式的第一版")
        .await
        .expect("Bot 发消息");
    chat.timeline
        .until("时间线里有这条", |item| text(item, "流式的第一版"))
        .await;
    let event = chat
        .timeline
        .items
        .iter()
        .find_map(|item| match &item.content {
            TimelineContent::Message { message } => message.event_id.clone(),
            _ => None,
        })
        .expect("消息的事件 ID");

    // 连着改三次（流式回复就是这样），再回应、再撤回另一条。
    for round in 1..=3 {
        let body = format!("流式的第{}版", round + 1);
        chat.bot_room
            .send_raw_message(serde_json::json!({
                "msgtype": "m.text",
                "body": format!("* {body}"),
                "m.new_content": {"msgtype": "m.text", "body": body},
                "m.relates_to": {"rel_type": "m.replace", "event_id": event},
            }))
            .await
            .expect("编辑");
    }
    chat.bot_room
        .send_raw_event(
            "m.reaction",
            serde_json::json!({
                "m.relates_to": {"rel_type": "m.annotation", "event_id": event, "key": "👍"},
            }),
        )
        .await
        .expect("回应");
    let other = chat
        .bot_room
        .send_raw_message(serde_json::json!({"msgtype": "m.text", "body": "马上撤回"}))
        .await
        .expect("再发一条");
    chat.bot_room
        .redact(other.as_ref(), "撤回")
        .await
        .expect("撤回");

    chat.timeline
        .until("最后的编辑到了", |item| text(item, "流式的第4版"))
        .await;
    assert!(
        !chat.timeline.was_cleared(),
        "编辑、回应、撤回不该重建整条时间线；重建原因：{}",
        chat.timeline.cleared_reasons()
    );
}

/// 服务器丢了房间列表那条连接的位置（`M_UNKNOWN_POS`，负载高时 Tuwunel 实测会回）：SDK 从头同步，打开着的房间只剩最新
/// 的一段。新消息照常到，往回翻能把更早的找回来。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_open_room_gets_its_earlier_messages_back_after_the_sync_starts_over() {
    let mut chat = Chat::open().await;
    chat.bot_room.send_text("之前的").await.expect("Bot 发消息");
    chat.timeline
        .until("之前的", |item| text(item, "之前的"))
        .await;

    chat.bot_room
        .send_text("同一批的")
        .await
        .expect("Bot 发消息");
    chat.timeline
        .until("同一批的", |item| text(item, "同一批的"))
        .await;
    // 订阅保留后，重启应答可能与缓存重叠而无需清空。断开期间发出超过订阅窗口的事件，
    // 让重启应答与原缓存不重叠，确定地覆盖清空后往回翻的路径。
    let limit = chat
        .home
        .sync_requests()
        .into_iter()
        .filter(|request| request["conn_id"] == "room-list")
        .rev()
        .find_map(|request| {
            request["room_subscriptions"][chat.room_id.as_str()]["timeline_limit"].as_u64()
        })
        .expect("opened room subscription must specify a timeline limit");
    chat.home.set_matrix_down(true);
    chat.sync_failed().await;
    for i in 0..=limit {
        chat._bot
            .send_text_to(&chat.room_id, &format!("gap message {i}"))
            .await
            .expect("Bot sends a message while the phone is disconnected");
    }
    chat.home.expire_room_list_sync();
    chat.home.set_matrix_down(false);
    chat.phone.dispatch(Action::AppForegrounded).unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    while !chat.home.room_list_sync_expired() {
        assert!(
            Instant::now() < deadline,
            "房间列表的同步一直没发下一个请求"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    chat.timeline.until_cleared().await;
    chat.bot_room.send_text("之后的").await.expect("Bot 发消息");

    chat.timeline
        .until_items("新消息到了，更早的也翻回来了", |items| {
            let has = |body| items.iter().any(|item| text(item, body).is_some());
            (has("之前的") && has("同一批的") && has("之后的")).then_some(())
        })
        .await;
    assert!(
        chat.timeline.was_cleared(),
        "同步没有从头开始，这条测试没测到；重建原因：{}",
        chat.timeline.cleared_reasons()
    );
}
