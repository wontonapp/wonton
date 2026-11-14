//! 测试里的时间线：按顺序应用每一批增量，和壳那一侧做的一样。

use std::sync::Arc;
use std::time::{Duration, Instant};
use wonton_protocol::{TimelineAction, TimelineTarget};

use wonton_client::{
    Action, BackPagination, ClientCore, KeyNotice, TimelineDiff, TimelineItem, TimelineScroll,
    TimelineSubscription, TimelineViewport,
};

use super::support::{Phone, WAIT, take_rebuild_reason};

/// 壳那一侧维护的时间线：按顺序应用每一批增量。同步从头开始时（服务器回 `M_UNKNOWN_POS`）SDK 只留最新的一段，
/// 列表被清空重来；壳这时露出顶上那一行，自己往回翻，这里也一样。
pub struct Timeline {
    pub items: Vec<TimelineItem>,
    subscription: Arc<TimelineSubscription>,
    core: Arc<ClientCore>,
    room_id: String,
    /// 旁聊的话题根；主对话是 `None`。
    thread_root: Option<String>,
    /// 核心给的旁聊头部（只在旁聊的时间线里有）。
    pub side_chat: Option<wonton_protocol::SideChatCard>,
    back_pagination: BackPagination,
    /// 最近一批的核心给的位置，壳按它画和滚。
    viewport: TimelineViewport,
    /// 壳现在向本订阅报告的位置。
    scroll: TimelineScroll,
    /// 核心给的加密入口，和壳一样逐批换。
    pub notice: Option<KeyNotice>,
    /// 最近一批里这个房间的确认卡片（ADR-0016），和壳一样逐批换。
    pub confirmations: Vec<wonton_client::ConfirmationView>,
    dispatched: usize,
    started: bool,
    cleared: bool,
    /// 每次清空的原因，按发生顺序；断言失败时带进 panic 消息。
    cleared_reasons: Vec<String>,
    paging: bool,
    /// 已经翻到房间开头。
    at_start: bool,
    /// 上一次往回翻的时刻，翻页按这个限速。
    last_page: Instant,
}

/// 往回翻的间隔：卡着的时候连续翻不会更快，太密只会堆请求。
const PAGE_EVERY: Duration = Duration::from_millis(250);

impl Timeline {
    pub async fn open(phone: &Phone, room_id: &str) -> Self {
        Self::open_target(phone, room_id, None).await
    }

    /// 打开主对话（`thread_root` 为 `None`）或一个旁聊的时间线。
    pub async fn open_target(phone: &Phone, room_id: &str, thread_root: Option<&str>) -> Self {
        let subscription = phone
            .subscribe_timeline(TimelineTarget {
                room_id: room_id.to_owned(),
                thread_root: thread_root.map(str::to_owned),
            })
            .await
            .expect("订阅时间线");
        Self {
            items: Vec::new(),
            subscription,
            core: phone.core.clone(),
            room_id: room_id.to_owned(),
            thread_root: thread_root.map(str::to_owned),
            side_chat: None,
            back_pagination: BackPagination::Idle,
            viewport: TimelineViewport::Bottom,
            scroll: TimelineScroll::Bottom,
            notice: None,
            confirmations: Vec::new(),
            dispatched: 0,
            started: false,
            cleared: false,
            cleared_reasons: Vec::new(),
            paging: false,
            at_start: false,
            last_page: Instant::now(),
        }
    }

    /// 应用一批增量。首批之后的 `Clear`、`Reset` 是列表被清空重来。
    fn apply_update(&mut self, update: wonton_client::TimelineUpdate) {
        self.viewport = update.viewport;
        self.notice = update.key_notice;
        self.side_chat = update.side_chat;
        self.confirmations = update.confirmations;
        let diffs = update.diffs;
        let back_pagination = update.back_pagination;
        // 翻页开始了，或者翻回来的条目到了：这一页算翻过，下回还在顶上就再翻。
        if back_pagination != BackPagination::Idle || !diffs.is_empty() {
            self.paging = false;
        }
        for diff in diffs {
            if self.started {
                match &diff {
                    TimelineDiff::Clear => {
                        self.cleared = true;
                        let reason = take_rebuild_reason(&self.room_id);
                        self.cleared_reasons.push(reason);
                        // 列表换了，翻到一半的那页算完，新列表要重新从头翻。
                        self.paging = false;
                        self.at_start = false;
                    }
                    TimelineDiff::Reset { .. } => {
                        self.cleared = true;
                        self.paging = false;
                        self.at_start = false;
                    }
                    _ => {}
                }
            }
            apply(&mut self.items, diff);
        }
        self.started = true;
        self.back_pagination = back_pagination;
        self.at_start = back_pagination == BackPagination::ReachedStart;
    }

    fn target(&self) -> TimelineTarget {
        TimelineTarget {
            room_id: self.room_id.clone(),
            thread_root: self.thread_root.clone(),
        }
    }

    /// 最近一批核心给的位置。
    pub fn viewport(&self) -> TimelineViewport {
        self.viewport
    }

    pub fn cancel(&self) {
        self.subscription.cancel();
    }

    pub async fn is_closed(&self) -> bool {
        self.subscription.next().await.is_none()
    }

    /// 等下一批，应用它，并把它本身交回去：断言这一批里给的位置和落点。
    pub async fn next(&mut self) -> wonton_client::TimelineUpdate {
        let update = self.subscription.next().await.expect("时间线订阅提前结束");
        self.apply_update(update.clone());
        update
    }

    /// 像壳一样报告列表滚到哪儿，再等核心按新位置送一批。位置没变时核心不会送，调用方自己保证变了。
    /// 队列里可能还有报着旧位置的批次，等到位置真的变成报告的那样为止。
    pub async fn report_scroll(&mut self, scroll: TimelineScroll) {
        self.scroll = scroll;
        self.subscription.set_scroll(scroll);
        let waiting = async {
            loop {
                let arrived = match scroll {
                    TimelineScroll::Bottom => self.viewport == TimelineViewport::Bottom,
                    TimelineScroll::Away => matches!(self.viewport, TimelineViewport::Away { .. }),
                };
                if arrived {
                    return;
                }
                self.next().await;
            }
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("报告 {scroll:?} 之后核心没按新位置送一批"));
    }

    pub fn was_cleared(&self) -> bool {
        self.cleared
    }

    /// 先观察重建，不触发回翻，保证后续新消息发生在重建之后。
    pub async fn until_cleared(&mut self) {
        let waiting = async {
            while !self.cleared {
                let update = self
                    .subscription
                    .next()
                    .await
                    .expect("timeline subscription ended");
                self.apply_update(update);
            }
        };
        match tokio::time::timeout(WAIT, waiting).await {
            Ok(()) => {}
            Err(_) => panic!(
                "同步重启后一直没重建时间线；清空原因 {:?}",
                self.cleared_reasons
            ),
        }
    }

    /// 每次清空的原因，按发生顺序；产品没打或没抓到原因时写“未记录”。
    pub fn cleared_reasons(&self) -> String {
        self.cleared_reasons.join("；")
    }

    /// 还没翻到头、没在翻这一页、离上一页够远了：像壳一样再翻一页。重启后同步从头开始，时间线只留最新一段，
    /// 壳这时也往回翻，不靠先收到一次清空——清空可能在时间线打开前就发生过。重建中的时间线一直报“正在翻”，
    /// 所以不拿状态当能不能翻的依据，只认到了房间开头。
    fn page_back_if_cleared(&mut self) {
        if !self.at_start && !self.paging && self.last_page.elapsed() >= PAGE_EVERY {
            self.paging = true;
            self.last_page = Instant::now();
            self.dispatched += 1;
            self.core
                .dispatch(Action::Timeline {
                    action: TimelineAction::PaginateBackwards {
                        target: self.target(),
                    },
                })
                .expect("往回翻");
        }
    }

    /// 应用增量，直到列表里有满足条件的条目。
    pub async fn until<R>(
        &mut self,
        what: &str,
        mut check: impl FnMut(&TimelineItem) -> Option<R>,
    ) -> R {
        self.until_items(what, |items| items.iter().find_map(&mut check))
            .await
    }

    /// 应用增量，直到核心给的加密入口变成 `wanted`（解密失败时给的入口、拿到密钥后收起）。
    pub async fn until_notice(&mut self, what: &str, wanted: Option<KeyNotice>) {
        let waiting = async {
            loop {
                if self.notice == wanted {
                    return;
                }
                let update = self.subscription.next().await.expect("时间线订阅提前结束");
                self.apply_update(update);
            }
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "{what}：{WAIT:?} 内入口没有变成 {wanted:?}，现在是 {:?}",
                    self.notice
                )
            });
    }

    /// 应用增量，直到这个房间的确认卡片（ADR-0016）满足条件。空批也带着当前的列表，变化就会醒。
    pub async fn until_confirmations<R>(
        &mut self,
        what: &str,
        mut check: impl FnMut(&[wonton_client::ConfirmationView]) -> Option<R>,
    ) -> R {
        let waiting = async {
            loop {
                if let Some(found) = check(&self.confirmations) {
                    return found;
                }
                let update = self.subscription.next().await.expect("时间线订阅提前结束");
                self.apply_update(update);
            }
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "{what}：{WAIT:?} 内确认卡片没有等到，现在是 {:#?}",
                    self.confirmations
                )
            })
    }

    /// 应用增量，直到整个列表满足条件。
    pub async fn until_items<R>(
        &mut self,
        what: &str,
        mut check: impl FnMut(&[TimelineItem]) -> Option<R>,
    ) -> R {
        let waiting = async {
            loop {
                if let Some(found) = check(&self.items) {
                    return found;
                }
                self.page_back_if_cleared();
                // 翻页按间隔限速：等太久没动静时也醒一次再翻，不能只等增量。
                let rest = PAGE_EVERY.saturating_sub(self.last_page.elapsed());
                tokio::select! {
                    update = self.subscription.next() => {
                        let update = update.expect("时间线订阅提前结束");
                        self.apply_update(update);
                    }
                    () = tokio::time::sleep(rest) => {}
                }
            }
        };
        match tokio::time::timeout(WAIT, waiting).await {
            Ok(found) => found,
            Err(_) => panic!(
                "{what}：{WAIT:?} 内没有等到，翻页 {:?}、翻过 {} 次、清空原因 {:?}，当前：{:#?}",
                self.back_pagination, self.dispatched, self.cleared_reasons, self.items
            ),
        }
    }
}

fn apply(items: &mut Vec<TimelineItem>, diff: TimelineDiff) {
    match diff {
        TimelineDiff::Append { items: more } => items.extend(more),
        TimelineDiff::Clear => items.clear(),
        TimelineDiff::PushFront { item } => items.insert(0, item),
        TimelineDiff::PushBack { item } => items.push(item),
        TimelineDiff::PopFront => {
            items.remove(0);
        }
        TimelineDiff::PopBack => {
            items.pop();
        }
        TimelineDiff::Insert { index, item } => items.insert(index as usize, item),
        TimelineDiff::Set { index, item } => items[index as usize] = item,
        TimelineDiff::Remove { index } => {
            items.remove(index as usize);
        }
        TimelineDiff::Truncate { length } => items.truncate(length as usize),
        TimelineDiff::Reset { items: all } => *items = all,
    }
}
