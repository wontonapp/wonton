//! 时间线的位置（ADR-0071）：翻走期间来的消息数、回到底部、重进房间停在已读标记，都由核心算。

use std::time::Duration;

use wonton_client::{Action, RoomMembership, TimelineContent, TimelineScroll, TimelineViewport};
use wonton_matrix::{ChatRoom, MatrixSession, OwnedRoomId};
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::accept_the_bots_invite;
use super::support::{Home, Phone, Timeline, WAIT};
use super::text;

/// 和 Bot 的私聊，以及主人的手机核心。
struct Chat {
    phone: Phone,
    home: Home,
    /// 只为了保住 `bot_room`。
    _bot: MatrixSession,
    bot_room: ChatRoom,
    room_id: OwnedRoomId,
    dir: tempfile::TempDir,
}

impl Chat {
    async fn open(dir: tempfile::TempDir) -> Self {
        let phone = Phone::open(dir.path());
        let home = Home::start(&[phone.device_id()]).await;
        phone.connect(&home);
        phone.signed_in().await;
        let bot = home.bot("hermes", "Hermes").await;
        let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
        let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
        Self {
            phone,
            home,
            _bot: bot,
            bot_room,
            room_id,
            dir,
        }
    }

    async fn timeline(&self) -> Timeline {
        Timeline::open(&self.phone, self.room_id.as_str()).await
    }

    async fn bot_says(&self, body: &str) {
        self.bot_room.send_text(body).await.expect("Bot 发消息");
    }
    async fn next_message(
        &self,
        timeline: &mut Timeline,
        body: &str,
    ) -> wonton_client::TimelineUpdate {
        tokio::time::timeout(WAIT, async {
            loop {
                let update = timeline.next().await;
                if timeline.items.iter().any(|item| text(item, body).is_some()) {
                    return update;
                }
            }
        })
        .await
        .expect("消息到达时间线")
    }

    /// 聊天列表那一行现在的未读数。
    fn unread(&self) -> u64 {
        self.phone
            .chat()
            .rooms
            .iter()
            .find(|row| row.room_id == self.room_id.as_str())
            .map_or(0, |row| row.unread_count)
    }

    /// 等未读涨到至少 `count`。未读数是重新数出来的，可能从 0 直接跳到 3，不逐个数。
    async fn wait_unread(&self, count: u64) {
        self.phone
            .wait_chat("未读数涨了", |view| {
                view.rooms
                    .iter()
                    .any(|row| row.room_id == self.room_id.as_str() && row.unread_count >= count)
                    .then_some(())
            })
            .await;
    }

    fn target(&self) -> TimelineTarget {
        TimelineTarget {
            room_id: self.room_id.to_string(),
            thread_root: None,
        }
    }

    /// 每次调用前都有新消息：先等未读出现，再等回执把它清零，避免把同步前的初始 0 当成完成。
    async fn read(&self) {
        self.wait_unread(1).await;
        self.phone
            .dispatch(Action::Timeline {
                action: TimelineAction::SetTimelineScroll {
                    target: self.target(),
                    scroll: TimelineScroll::Bottom,
                },
            })
            .unwrap();
        self.phone
            .dispatch(Action::Timeline {
                action: TimelineAction::MarkRead {
                    target: self.target(),
                },
            })
            .unwrap();
        self.phone
            .wait_chat("已读清零", |view| {
                view.rooms
                    .iter()
                    .any(|row| row.room_id == self.room_id.as_str() && row.unread_count == 0)
                    .then_some(())
            })
            .await;
    }

    /// 杀掉核心再起来：同一个数据目录，恢复上次的会话。
    async fn restart(self) -> Self {
        let Self {
            phone,
            home,
            _bot: bot,
            bot_room,
            room_id,
            dir,
        } = self;
        drop(phone);
        let phone = Phone::open(dir.path());
        phone.connect(&home);
        phone.signed_in().await;
        phone
            .wait_chat("重启后同步到私聊", |view| {
                view.rooms
                    .iter()
                    .any(|row| {
                        row.room_id == room_id.as_str() && row.membership == RoomMembership::Joined
                    })
                    .then_some(())
            })
            .await;
        Self {
            phone,
            home,
            _bot: bot,
            bot_room,
            room_id,
            dir,
        }
    }
}

/// 旧壳或排队中的回调仍发 MarkRead 时，核心以最后报告的位置拒绝错误已读。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn stale_read_requests_do_not_advance_a_scrolled_away_room() {
    let chat = Chat::open(tempfile::tempdir().unwrap()).await;
    let mut timeline = chat.timeline().await;
    chat.bot_says("先看完").await;
    timeline
        .until("先来的消息到了", |item| text(item, "先看完"))
        .await;
    timeline.report_scroll(TimelineScroll::Bottom).await;
    chat.read().await;
    timeline.report_scroll(TimelineScroll::Away).await;
    chat.bot_says("尚未读到").await;
    let update = chat.next_message(&mut timeline, "尚未读到").await;
    assert!(!update.scroll_to_bottom);
    chat.wait_unread(1).await;
    chat.phone
        .dispatch(Action::Timeline {
            action: TimelineAction::MarkRead {
                target: chat.target(),
            },
        })
        .unwrap();
    // 重开后服务器已读落点仍在新消息之前，不能只断言异步回执尚未到达时的旧数字。
    drop(timeline);
    let mut reopened = chat.timeline().await;
    let first = reopened.next().await;
    assert_ne!(
        first.anchor,
        reopened.items.last().map(|item| item.id.clone())
    );
    assert!(matches!(
        first.viewport,
        TimelineViewport::Away { new_messages: 1 }
    ));
    reopened.report_scroll(TimelineScroll::Bottom).await;
    chat.read().await;
    assert_eq!(chat.unread(), 0);
}

/// 翻走时自己的新消息才要求壳回到底部，消息的发送状态和编辑更新不重复强制滚动。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_own_message_requests_the_bottom_while_reading_history() {
    let chat = Chat::open(tempfile::tempdir().unwrap()).await;
    let mut timeline = chat.timeline().await;
    chat.bot_says("历史").await;
    timeline.until("历史到了", |item| text(item, "历史")).await;
    timeline.report_scroll(TimelineScroll::Away).await;
    chat.phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: chat.target(),
                body: "回到底部看自己发的".into(),
                reply_to: None,
            },
        })
        .unwrap();
    let update = chat.next_message(&mut timeline, "回到底部看自己发的").await;
    assert!(update.scroll_to_bottom);
    assert!(matches!(update.viewport, TimelineViewport::Away { .. }));
}

/// 翻走以后来的消息只让数字变大，列表不跟着滚；报到底以后数字归零，已读回执照旧等壳在到底时发。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn new_messages_only_raise_the_number_until_the_shell_reports_the_bottom() {
    let chat = Chat::open(tempfile::tempdir().unwrap()).await;
    let mut timeline = chat.timeline().await;
    for body in ["先来一条", "再来一条"] {
        chat.bot_says(body).await;
        timeline
            .until("先来的消息到了", |item| text(item, body))
            .await;
    }
    chat.wait_unread(2).await;
    drop(timeline);

    // 两条已加载的未读让重进落点在最新一条之前，固定首批 Away，不能误当成之后翻走的回报。
    let mut timeline = chat.timeline().await;
    timeline.next().await;
    assert_eq!(
        timeline.viewport(),
        TimelineViewport::Away { new_messages: 2 }
    );
    timeline.report_scroll(TimelineScroll::Bottom).await;
    chat.read().await;

    timeline.report_scroll(TimelineScroll::Away).await;
    assert_eq!(
        timeline.viewport(),
        TimelineViewport::Away { new_messages: 0 },
        "还没有新消息，按钮只画箭头；时间线 {:?}，聊天列表未读 {}",
        timeline.viewport(),
        chat.unread()
    );

    for body in ["一", "二", "三"] {
        chat.bot_says(body).await;
    }
    timeline
        .until("三条新消息都到了", |item| text(item, "三"))
        .await;
    chat.wait_unread(2).await;
    until_the_number_matches(&mut timeline, &chat).await;
    assert!(
        chat.unread() >= 2,
        "三条里至少两条进数字，房间现在说 {}",
        chat.unread()
    );

    timeline.report_scroll(TimelineScroll::Bottom).await;
    assert_eq!(timeline.viewport(), TimelineViewport::Bottom, "到底了");
    let unread = chat.unread();
    assert!(unread >= 2, "报到底本身不挪已读位置，未读还是 {unread}");
    chat.read().await;
    assert_eq!(chat.unread(), 0);
}

/// 等核心报的数字和聊天列表那一行的读数一致。未读数与条数可能差一两条，两边同源即可。
///
/// 每轮先比状态再等事件：`next()` 可能在上一次状态变化之后才挂起，一直等下去就赶不上已发生的匹配
/// （托管 runner 上实测过），所以 `next()` 外面套一层短超时，醒了先重新比一次。
async fn until_the_number_matches(timeline: &mut Timeline, chat: &Chat) {
    let waiting = async {
        loop {
            let wanted = TimelineViewport::Away {
                new_messages: u32::try_from(chat.unread()).unwrap_or(u32::MAX),
            };
            if timeline.viewport() == wanted {
                return;
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), timeline.next()).await;
        }
    };
    if tokio::time::timeout(WAIT, waiting).await.is_err() {
        panic!(
            "数字一直跟不上聊天列表那一行：行 {}、位置 {:?}",
            chat.unread(),
            timeline.viewport()
        );
    }
}

/// 有未读时重进房间：首批的落点是已读标记（“以下是新消息”），不是最新一条；位置是翻走的数字。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn reopening_with_unread_stops_at_the_read_marker() {
    let chat = Chat::open(tempfile::tempdir().unwrap()).await;
    chat.bot_says("先来一条").await;
    let mut timeline = chat.timeline().await;
    timeline
        .until("先来的消息到了", |item| text(item, "先来一条"))
        .await;
    chat.read().await;
    for body in ["一", "二", "三"] {
        chat.bot_says(body).await;
    }
    chat.wait_unread(3).await;
    drop(timeline);

    let mut timeline = chat.timeline().await;
    let first = timeline.next().await;
    assert_eq!(first.anchor.as_deref(), Some("__read_marker"));
    assert!(
        timeline
            .items
            .iter()
            .any(|item| item.id == "__read_marker" && item.content == TimelineContent::ReadMarker),
        "落点是列表里的已读标记"
    );
    assert_eq!(first.viewport, TimelineViewport::Away { new_messages: 3 });
}

/// 没有未读时重进：落点是最新一条，列表贴着底部。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn reopening_without_unread_stops_at_the_latest_message() {
    let chat = Chat::open(tempfile::tempdir().unwrap()).await;
    chat.bot_says("只有这一条").await;
    let mut timeline = chat.timeline().await;
    timeline
        .until("消息到了", |item| text(item, "只有这一条"))
        .await;
    chat.read().await;
    drop(timeline);

    let mut timeline = chat.timeline().await;
    let first = timeline.next().await;
    let latest = timeline.items.last().expect("列表里有最新一条").id.clone();
    assert_eq!(first.anchor.as_deref(), Some(latest.as_str()));
    assert_eq!(first.viewport, TimelineViewport::Bottom);
}

/// 重启核心再进：已读位置在服务器上，落点还是已读标记，未读数照旧。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_restart_still_stops_at_the_read_marker() {
    let chat = Chat::open(tempfile::tempdir().unwrap()).await;
    chat.bot_says("先来一条").await;
    let mut timeline = chat.timeline().await;
    timeline
        .until("先来的消息到了", |item| text(item, "先来一条"))
        .await;
    chat.read().await;
    for body in ["一", "二", "三"] {
        chat.bot_says(body).await;
    }
    chat.wait_unread(3).await;
    drop(timeline);

    let chat = chat.restart().await;
    let mut timeline = chat.timeline().await;
    let first = timeline.next().await;
    assert_eq!(first.anchor.as_deref(), Some("__read_marker"));
    assert_eq!(first.viewport, TimelineViewport::Away { new_messages: 3 });
}
