//! 测试里的时间线：按顺序应用每一批增量，和壳那一侧做的一样。

use std::sync::Arc;

use wonton_client::{
    Action, BackPagination, ClientCore, TimelineDiff, TimelineItem, TimelineSubscription,
};

use super::support::{Phone, WAIT, take_rebuild_reason};

/// 壳那一侧维护的时间线：按顺序应用每一批增量。同步从头开始时（服务器回 `M_UNKNOWN_POS`）SDK 只留最新的一段，
/// 列表被清空重来；壳这时露出顶上那一行，自己往回翻，这里也一样。
pub struct Timeline {
    pub items: Vec<TimelineItem>,
    subscription: Arc<TimelineSubscription>,
    core: Arc<ClientCore>,
    room_id: String,
    back_pagination: BackPagination,
    dispatched: usize,
    started: bool,
    cleared: bool,
    /// 每次清空的原因，按发生顺序；断言失败时带进 panic 消息。
    cleared_reasons: Vec<String>,
    paging: bool,
}

impl Timeline {
    pub async fn open(phone: &Phone, room_id: &str) -> Self {
        let subscription = phone
            .subscribe_timeline(room_id.to_owned())
            .await
            .expect("订阅时间线");
        Self {
            items: Vec::new(),
            subscription,
            core: phone.core.clone(),
            room_id: room_id.to_owned(),
            back_pagination: BackPagination::Idle,
            dispatched: 0,
            started: false,
            cleared: false,
            cleared_reasons: Vec::new(),
            paging: false,
        }
    }

    /// 应用一批增量。首批之后的 `Clear`、`Reset` 是列表被清空重来。
    fn apply_update(&mut self, diffs: Vec<TimelineDiff>, back_pagination: BackPagination) {
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
                    }
                    TimelineDiff::Reset { .. } => self.cleared = true,
                    _ => {}
                }
            }
            apply(&mut self.items, diff);
        }
        self.started = true;
        self.back_pagination = back_pagination;
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
                self.apply_update(update.diffs, update.back_pagination);
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

    /// 清空过、还没翻到头、没在翻：像壳一样再翻一页。
    fn page_back_if_cleared(&mut self) {
        if self.cleared && !self.paging && self.back_pagination == BackPagination::Idle {
            self.paging = true;
            self.dispatched += 1;
            self.core
                .dispatch(Action::PaginateBackwards {
                    room_id: self.room_id.clone(),
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
                let update = self.subscription.next().await.expect("时间线订阅提前结束");
                self.apply_update(update.diffs, update.back_pagination);
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
