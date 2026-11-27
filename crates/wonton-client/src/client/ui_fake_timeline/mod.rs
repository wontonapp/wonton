//! 假数据入口（`ui_fake.rs`）里房间时间线对壳操作的响应：界面测试点引用跳转、“向下”回去、点选项按钮、
//! 发“/”命令时，假房间照 ADR-0109 再推一批。第一批的条目在 `ui_fake_room/`。
//!
//! 不是核心的实现：没有翻页、没有回执，只认测试约定的那几个操作。组内位置用核心的真规则（[`Runs`](crate::chat::Runs)）补。
//!
//! 走查“回到底部”上的未读数字：壳报告翻走以后，后台陆续来三条（[`ARRIVALS`](unread::ARRIVALS)），回到底部清掉。

mod actions;
mod compose;
mod jump;
mod subscriptions;
#[cfg(test)]
mod test_support;
mod timeline;
mod unread;

use std::sync::Mutex;

use super::ui_fake_room;

pub(crate) use subscriptions::FakeTimelines;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
