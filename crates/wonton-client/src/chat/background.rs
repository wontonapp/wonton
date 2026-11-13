//! 保持连接下的后台通知（ADR-0003 的“设计：保持连接”）：App 在后台时，同步到的、按推送规则该通知的
//! 消息收集进 `ChatViewModel::background_notifications`，回到前台清空；壳照它逐条弹本地通知。缺密钥的
//! 占位在密钥晚到时由核心回填，壳用同一个通知标识把占位换成正文。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::task::JoinHandle;
use wonton_matrix::{BackgroundNotification, MatrixSession, UserId};
use wonton_protocol::{PushContent, PushMethod};

use super::unread_total;
use crate::push::push_notification;

use super::ChatLink;

/// 后台通知的上限。几天不打开 App 时同步一直跑着，积压没有边，不能跟着消息数涨；50 条足够铺满通知栏，
/// 更早的打开 App 后从房间列表和时间线看。
const BACKGROUND_NOTIFICATIONS: usize = 50;

impl ChatLink {
    /// App 回到前台（`AppForegrounded`）：和网络恢复共用的重登逻辑之外，还停止收集并清空后台通知——
    /// 主人看得见界面了。网络变化在后台也可能报（`NetworkChanged` 走 [`ChatLink::foregrounded`]），
    /// 不能当成回到前台。
    pub(crate) fn app_foregrounded(self: &Arc<Self>) {
        self.bots.foregrounded();
        self.background.store(false, Ordering::SeqCst);
        // 后台收集的通知都弹过了，主人现在看得见界面。
        self.view
            .send_modify(|view| view.background_notifications.clear());
        self.foregrounded();
    }

    /// App 进了后台：保持连接下同步到的该通知的消息开始收集，壳照 `background_notifications` 弹系统通知。
    pub(crate) fn backgrounded(&self) {
        self.bots.backgrounded();
        self.background.store(true, Ordering::SeqCst);
    }

    /// 把后台通知的回调接到 ViewModel 上，挂在会话的任务列表里：会话换掉时任务和它一起停。
    pub(crate) fn observe_background(
        self: &Arc<Self>,
        session: &Arc<MatrixSession>,
    ) -> JoinHandle<()> {
        let session = session.clone();
        let chat = self.clone();
        let own = session.user_id().to_owned();
        self.runtime.spawn(async move {
            session
                .register_background_notifications(move |event| {
                    chat.background_notification(&own, event)
                })
                .await;
        })
    }

    /// 保持连接下同步到的一条该通知的消息：不收自己发的，只在 App 在后台、方式是保持连接时收集；
    /// 按 `event_id` 去重（同一条事件可能同步到两次），超过上限丢最早的。先到的在前。缺密钥的占位在
    /// 密钥晚到、正文回填时换成正文，壳据此用同一个通知标识重弹。
    fn background_notification(self: &Arc<Self>, own: &UserId, event: BackgroundNotification) {
        tracing::info!(
            sender = %event.message.sender,
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
