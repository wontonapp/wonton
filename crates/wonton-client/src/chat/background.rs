//! 保持连接下的后台通知（ADR-0003 的“设计：保持连接”）：App 在后台时，同步到的、按推送规则该通知的
//! 消息收集进 `ChatViewModel::background_notifications`，回到前台清空；壳照它逐条弹本地通知。缺密钥的
//! 占位在密钥晚到时由核心回填，壳用同一个通知标识把占位换成正文。

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use tokio::task::JoinHandle;
use wonton_matrix::{BackgroundNotification, MatrixSession, UserId};
use wonton_protocol::{PushContent, PushMethod, RoomRow};

use super::unread_total;
use crate::push::push_notification;

use super::ChatLink;
use super::util::lock;

/// 后台通知的上限。几天不打开 App 时同步一直跑着，积压没有边，不能跟着消息数涨；50 条足够铺满通知栏，
/// 更早的打开 App 后从房间列表和时间线看。
const BACKGROUND_NOTIFICATIONS: usize = 50;

/// 进后台那一刻每个房间的分界：该房间「别人发的」最新一条的时间（Unix 毫秒）。本地回显是我自己发的，
/// 时间是手机时钟，可能比服务器快几十秒；把回显算进来会把分界推到服务器时间的前面，别人的回复就被当成
/// 旧消息吞掉。没有分界的房间（没预览、只有自己的消息、或冷启动）什么都不挡。
#[derive(Default)]
pub(super) struct NotifyBoundary(Mutex<HashMap<String, u64>>);

impl NotifyBoundary {
    /// 按当前房间列表给每个房间记分界。
    fn arm(&self, rooms: &[RoomRow]) {
        let mut boundary = lock(&self.0);
        boundary.clear();
        for room in rooms {
            // 本地回显是手机时钟打的，可能比服务器快几十秒，算进来会把别人的回复挡成旧消息。
            let Some(preview) = room.preview.as_ref().filter(|preview| !preview.is_own) else {
                continue;
            };
            let bound = boundary.entry(room.room_id.clone()).or_default();
            *bound = (*bound).max(preview.timestamp_ms);
        }
    }

    fn disarm(&self) {
        lock(&self.0).clear();
    }

    /// 比该房间进后台时的分界新才算；没有分界、或事件没有时间时不挡。
    fn accepts(&self, room_id: &str, origin_server_ts: Option<u64>) -> bool {
        let boundary = lock(&self.0);
        match (boundary.get(room_id), origin_server_ts) {
            (Some(bound), Some(ts)) => ts > *bound,
            _ => true,
        }
    }
}

impl ChatLink {
    /// App 回到前台（`AppForegrounded`）：和网络恢复共用的重登逻辑之外，还停止收集并清空后台通知——
    /// 主人看得见界面了。网络变化在后台也可能报（`NetworkChanged` 走 [`ChatLink::foregrounded`]），
    /// 不能当成回到前台。
    pub(crate) fn app_foregrounded(self: &Arc<Self>) {
        self.bots.foregrounded();
        self.background.store(false, Ordering::SeqCst);
        self.notify_boundary.disarm();
        // 后台收集的通知都弹过了，主人现在看得见界面。
        self.view
            .send_modify(|view| view.background_notifications.clear());
        self.foregrounded();
    }

    /// App 进了后台：保持连接下同步到的该通知的消息开始收集，壳照 `background_notifications` 弹系统通知。
    pub(crate) fn backgrounded(&self) {
        // 进后台时每个房间已经知道的最新一条：比它还早的事件是看过的旧消息，首轮同步、`M_UNKNOWN_POS`
        // 重来和重放的响应会把它们再送一遍。分界只看别人发的消息：本地回显用的是手机时钟，手机比服务器
        // 快几十秒时会把别人的回复算成旧的。
        self.notify_boundary.arm(&self.view.borrow().rooms);
        self.bots.backgrounded();
        self.background.store(true, Ordering::SeqCst);
    }

    /// 这条事件要不要取回并弹通知：App 在后台、方式是保持连接，而且它比进后台时该房间的分界新。
    fn should_notify(&self, room_id: &str, origin_server_ts: Option<u64>) -> bool {
        if !self.background.load(Ordering::SeqCst) || *self.method.borrow() != PushMethod::Keepalive
        {
            return false;
        }
        self.notify_boundary.accepts(room_id, origin_server_ts)
    }

    /// 把后台通知的回调接到 ViewModel 上，挂在会话的任务列表里：会话换掉时任务和它一起停。
    pub(crate) fn observe_background(
        self: &Arc<Self>,
        session: &Arc<MatrixSession>,
    ) -> JoinHandle<()> {
        let session = session.clone();
        let chat = self.clone();
        let notify = self.clone();
        let own = session.user_id().to_owned();
        self.runtime.spawn(async move {
            session
                .register_background_notifications(
                    move |room_id, origin_server_ts| {
                        notify.should_notify(room_id, origin_server_ts)
                    },
                    move |event| chat.background_notification(&own, event),
                )
                .await;
        })
    }

    /// 保持连接下同步到的一条该通知的消息：不收自己发的，只在 App 在后台、方式是保持连接时收集；
    /// 按 `event_id` 去重（同一条事件可能同步到两次），超过上限丢最早的。先到的在前。缺密钥的占位在
    /// 密钥晚到、正文回填时换成正文，壳据此用同一个通知标识重弹。
    fn background_notification(self: &Arc<Self>, own: &UserId, event: BackgroundNotification) {
        tracing::info!(
            sender = %event.message.sender,
            room = %event.room_id,
            event = %event.event_id,
            background = self.background.load(Ordering::SeqCst),
            method = ?*self.method.borrow(),
            "background notification callback"
        );
        if event.message.sender == *own {
            return;
        }
        if !self.background.load(Ordering::SeqCst) || *self.method.borrow() != PushMethod::Keepalive
        {
            return;
        }
        // 契约（`PushNotification::server_id`）是本机槽位号（`ServerRow::id`），不是这台的 endpoint ID：
        // 壳点开通知按槽位号切服务器，端点 ID 在槽位列表里查不到，会被当成“已移除的服务器”误报。
        let Some(mut notification) = push_notification(
            Some(self.slot.clone()),
            own,
            &event.room_id,
            &event.event_id,
            event.message,
        ) else {
            return;
        };
        notification.lock_screen_private = self.lock_screen.borrow().hidden(notification.category);
        notification.unread_total = unread_total(&self.view.borrow());
        // App 锁着就不在通知里写发送者和正文（ADR-0102）。
        notification.hidden = self.app_lock.is_locked();
        self.view.send_modify(|view| {
            if let Some(shown) = view
                .background_notifications
                .iter_mut()
                .find(|shown| shown.event_id == notification.event_id)
            {
                // 同一条事件又到了：只有占位换正文这一种升级，重复的正文和再一次的占位都不动。
                if matches!(shown.content, PushContent::Unavailable { .. })
                    && !matches!(notification.content, PushContent::Unavailable { .. })
                {
                    *shown = notification;
                }
                return;
            }
            view.background_notifications.push(notification);
            if view.background_notifications.len() > BACKGROUND_NOTIFICATIONS {
                view.background_notifications.remove(0);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::NotifyBoundary;
    use wonton_protocol::{ChatLayout, MessageSummary, RoomMembership, RoomPreview, RoomRow};

    fn room(room_id: &str, own: bool, timestamp_ms: u64) -> RoomRow {
        RoomRow {
            room_id: room_id.to_owned(),
            name: "Hermes".to_owned(),
            is_direct: true,
            direct_user_id: Some("@hermes:wonton.localhost".to_owned()),
            plugin: None,
            bot_chat: None,
            is_encrypted: false,
            is_read_only: false,
            membership: RoomMembership::Joined,
            preview: Some(RoomPreview {
                sender_name: "Hermes".to_owned(),
                is_own: own,
                body: MessageSummary::Text {
                    text: "在吗".to_owned(),
                },
                timestamp_ms,
            }),
            side_chats: false,
            unread_count: 0,
            layout: ChatLayout::default(),
            call_active: false,
            unread_mentions: 0,
        }
    }

    /// 本地回显用的是手机时钟，手机比服务器快 30 秒；Bot 的回复是服务器时间，落在回显后面，不能被当成旧消息。
    #[test]
    fn own_echo_ahead_of_the_server_clock_does_not_hide_replies() {
        let boundary = NotifyBoundary::default();
        boundary.arm(&[room("!r:wonton.localhost", true, 1_030_000)]);
        assert!(
            boundary.accepts("!r:wonton.localhost", Some(1_000_000)),
            "服务器时间比回显早 30 秒的回复不该被挡"
        );
    }

    /// 分界按房间记：一个房间的时间不影响另一个房间；更早的同房间消息仍然挡住。
    #[test]
    fn old_messages_stay_out_and_rooms_have_their_own_boundary() {
        let boundary = NotifyBoundary::default();
        boundary.arm(&[
            room("!a:wonton.localhost", false, 1_000),
            room("!b:wonton.localhost", false, 1_000_000),
        ]);
        assert!(!boundary.accepts("!a:wonton.localhost", Some(1_000)));
        assert!(boundary.accepts("!a:wonton.localhost", Some(1_001)));
        assert!(boundary.accepts("!c:wonton.localhost", Some(1)));
    }
}
