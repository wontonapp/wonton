//! 收件箱（ADR-0119）：条目在核心里，按发送方、上限和生命周期存；壳只收 ViewModel、发回结构化动作。
//!
//! - 存储：`core/inbox/items/<id>.json` 和 `core/inbox/pages/<id>.html`，都是 0600。
//! - 条目由 MCP 总线的 `wonton__inbox_*` 工具、worker 事件入口和现有确认卡片投影写入。
//! - 核心不认具体 Agent（ADR-0015）：收件人就是 Bot 的归属人或管理员，按用户名认。
//! - 页面是发送时的快照；CSP 由 [`page::policy`] 按声明的域名生成，名单为空就完全断网。

mod page;
mod projection;
mod store;
pub(crate) mod tools;

pub use page::{FixedClock, PageTokens, policy};
pub use projection::{confirmation_id, project_confirmation};
pub use store::{
    AdminRoster, AdminSet, AnswerOutcome, Clock, CreateInput, CreateReply, DevicePresence, Inbox,
    InboxError, NewPage, SystemClock,
};
pub(crate) use tools::{call, definitions};

/// 现在几点（Unix 毫秒）。
pub fn now_ms() -> u64 {
    use crate::inbox::Clock;
    SystemClock.now_ms()
}

pub fn summarize_item(
    item: &wonton_protocol::InboxItem,
    viewer: &str,
) -> wonton_protocol::InboxListItem {
    store::summarize(item, viewer)
}

pub fn sort_list(items: &mut [wonton_protocol::InboxListItem]) {
    store::sort_list(items);
}

/// 一条条目在内存里能同时等几个阻塞调用方（同一个 dedupe 键被重复提交）。
pub(crate) const MAX_WAITERS_PER_ITEM: usize = 4;

#[cfg(test)]
mod tests;
