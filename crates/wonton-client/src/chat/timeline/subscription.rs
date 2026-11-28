//! 一个房间的时间线订阅：壳逐批拉走、报告位置、需要时取消。

use std::sync::Arc;

use tokio::runtime::Handle;
use tokio::sync::{Mutex, mpsc, watch};
use wonton_matrix::{ChatRoom, OwnedRoomId};
use wonton_protocol::{ConfirmationView, TimelineScroll, TimelineUpdate};

use super::super::jump::{self, Live};
use super::super::lock;
use super::super::menu::{Menus, Outbox};
use super::super::previews::Previews;
use super::super::side_chat::Support;
use super::super::target::TimelineOpening;
use super::forward::forward;

/// 一个房间的时间线订阅。壳循环调用 [`next`](Self::next)，把每一批增量按顺序应用到自己的列表上；不再需要时
/// 调用 [`cancel`](Self::cancel)。第一批是 `Reset`，给出当前的全部条目。
#[derive(uniffi::Object)]
pub struct TimelineSubscription {
    updates: Mutex<mpsc::UnboundedReceiver<TimelineUpdate>>,
    cancelled: watch::Sender<bool>,
    scroll: watch::Sender<TimelineScroll>,
    /// 按订阅建一次的正在输入、命令表和跳转状态（ADR-0109）；假数据入口没有，壳的操作由假房间推。
    pub(in crate::chat) live: std::sync::Mutex<Option<Arc<Live>>>,
    opening: std::sync::Mutex<Option<TimelineOpening>>,
    /// 假数据入口的订阅没有转发任务：壳报告的位置交给假房间，假房间再推一批（它也拿着发送端，通道不关）。
    #[cfg(feature = "ui-fake")]
    on_scroll: Option<Box<dyn Fn(TimelineScroll) + Send + Sync>>,
    #[cfg(feature = "ui-fake")]
    status: Option<Box<dyn Fn() -> String + Send + Sync>>,
}

/// 一个房间的时间线订阅要用的部件：房间、能显示的卡片和各路账号数据。[`TimelineSubscription::start`] 一次带走。
pub(crate) struct TimelineSource {
    pub(crate) room: Arc<ChatRoom>,
    /// 打开的房间：卡片行按 `origin.room_id` 等于它过滤（ADR-0016）。
    pub(crate) room_id: OwnedRoomId,
    /// 这个房间能显示的确认卡片。
    pub(crate) confirmations: watch::Receiver<Arc<Vec<ConfirmationView>>>,
    pub(crate) plugins: Arc<crate::plugins::PluginsLink>,
    /// 推送里带来的正文预览（ADR-0118），打开着房间时来了新的也补一批。
    pub(crate) push_previews: Arc<crate::push::PushPreviews>,
    /// 打开的是旁聊时是话题根：只补这个旁聊的预览。
    pub(crate) thread_root: Option<wonton_matrix::OwnedEventId>,
    pub(crate) previews: Previews,
    pub(crate) menus: Arc<std::sync::Mutex<Menus>>,
    pub(crate) room_view: watch::Receiver<Option<Arc<crate::pages::RoomView>>>,
    pub(crate) chat: watch::Receiver<wonton_protocol::ChatViewModel>,
    /// 各房间的旁聊支持声明（ADR-0093 §1）。
    pub(crate) support: Arc<Support>,
    /// Bot 名单的账本：「就绪」看已安装表和第一份活动报告都到了没有（[`super::super::bots::Bots::ready`]）。
    pub(in crate::chat) bots: Arc<super::super::bots::Bots>,
    /// 取房间状态用（命令表）。
    pub(crate) session: Arc<wonton_matrix::MatrixSession>,
}

impl TimelineSubscription {
    /// 在核心的运行时里开始转发 `room` 的时间线。订阅释放或取消后转发自动停止。
    ///
    /// `room_view` 是 tabs 合成的房间视图（顶栏的 widget 入口）：每一批都带上当前值，变了也发一批空的。`chat` 读加密状态，
    /// 决定解不开的消息旁给什么入口。`confirmations` 是这个房间能显示的确认卡片（ADR-0016），出口按
    /// `origin.room_id` 过滤后随每一批带上，变了也发一批空的。
    pub(crate) fn start(
        runtime: &Handle,
        source: TimelineSource,
        opening: TimelineOpening,
    ) -> Arc<Self> {
        let TimelineSource {
            room,
            room_id,
            confirmations,
            plugins,
            previews,
            menus,
            room_view,
            chat,
            support,
            bots: ledger,
            session,
            push_previews,
            thread_root,
        } = source;
        let (sender, receiver) = mpsc::unbounded_channel();
        let (cancelled, mut stop) = watch::channel(false);
        // 落点摆好并由壳确认之前，不能把首屏量出来的默认位置当成已经读到底。
        let (scroll, mut scrolled) = watch::channel(TimelineScroll::Away);
        let live = jump::live(runtime, &room, room_id.clone());
        let watched = live.clone();
        runtime.spawn(async move {
            let outbox = Outbox::new(
                sender,
                menus,
                room_view.clone(),
                chat.clone(),
                room_id,
                // 只读快照用的克隆。等变化必须用下面交给 `forward` 的那个接收者：克隆带着克隆那一刻的
                // 已读位置，每轮新克隆一个去等 `changed()` 会立刻返回，订阅就空转着一批接一批往外发。
                confirmations.clone(),
                watched.clone(),
            );
            tokio::select! {
                () = forward(
                    &room,
                    &plugins,
                    previews,
                    &outbox,
                    &mut scrolled,
                    room_view,
                    chat,
                    confirmations,
                    &support,
                    &ledger,
                    &watched,
                    &session,
                    &push_previews,
                    thread_root.as_ref(),
                ) => {}
                () = outbox.closed() => {}
                _ = stop.wait_for(|cancelled| *cancelled) => {}
            }
        });
        Arc::new(Self {
            updates: Mutex::new(receiver),
            cancelled,
            scroll,
            live: std::sync::Mutex::new(Some(live)),
            opening: std::sync::Mutex::new(Some(opening)),
            #[cfg(feature = "ui-fake")]
            on_scroll: None,
            #[cfg(feature = "ui-fake")]
            status: None,
        })
    }

    /// 假数据入口（`ClientCore::new_ui_fake`）的时间线：批次由假房间推进 `updates`，壳报告的位置交给 `on_scroll`。
    #[cfg(feature = "ui-fake")]
    pub(crate) fn seeded(
        updates: mpsc::UnboundedReceiver<TimelineUpdate>,
        on_scroll: Box<dyn Fn(TimelineScroll) + Send + Sync>,
        status: Box<dyn Fn() -> String + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            updates: Mutex::new(updates),
            cancelled: watch::Sender::new(false),
            scroll: watch::Sender::new(TimelineScroll::Away),
            live: std::sync::Mutex::new(None),
            opening: std::sync::Mutex::new(None),
            on_scroll: Some(on_scroll),
            status: Some(status),
        })
    }

    pub(crate) fn is_active(&self) -> bool {
        !*self.cancelled.borrow() && !self.cancelled.is_closed()
    }

    pub(crate) fn is_at_bottom(&self) -> bool {
        *self.scroll.borrow() == TimelineScroll::Bottom && !*self.cancelled.borrow()
    }
}

#[uniffi::export]
impl TimelineSubscription {
    /// 等待下一批增量。和 ViewModel 的订阅不同，这里的批次不会合并，壳要逐批应用。订阅取消、房间关闭或核心
    /// 释放后返回 `None`。
    pub async fn next(&self) -> Option<TimelineUpdate> {
        let mut cancelled = self.cancelled.subscribe();
        let mut updates = self.updates.lock().await;
        tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => None,
            update = updates.recv() => update,
        }
    }

    /// 壳直接向本订阅报告列表位置；同一目标的其他订阅不受影响。
    /// 跨过底部时发一次，下一批更新里带重新算过的位置和新消息数。
    pub fn set_scroll(&self, scroll: TimelineScroll) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.scrolled(scroll);
        }
        self.scroll.send_replace(scroll);
        #[cfg(feature = "ui-fake")]
        if let Some(on_scroll) = &self.on_scroll {
            on_scroll(scroll);
        }
    }

    /// 走查用：订阅和假时间线的当前状态，`observe start`、`timeline ended` 征进日志。
    pub fn debug_status(&self) -> String {
        let cancelled = *self.cancelled.borrow();
        #[cfg(feature = "ui-fake")]
        let extra = self
            .status
            .as_ref()
            .map_or_else(|| "live".into(), |status| status());
        #[cfg(not(feature = "ui-fake"))]
        let extra = "live";
        format!("cancelled={cancelled} {extra}")
    }

    /// 取消订阅，语义同 `HomeSubscription::cancel`（Swift 的 Task 取消不会传到 Rust）。
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
        drop(lock(&self.opening).take());
    }
}
