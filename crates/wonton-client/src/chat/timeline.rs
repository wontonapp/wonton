//! 打开的房间的时间线订阅：增量批次按顺序排队，不合并（ADR-0005）。
//!
//! 门面：对外只留订阅的入口，按职责分在下面的模块里——订阅本身、转发循环、批次与条目翻译、重建分页、
//! 重算和安静清空。

mod diffs;
mod forward;
mod push_body;
#[cfg(test)]
#[path = "timeline/push_body_tests.rs"]
mod push_body_tests;
mod rebuild;
mod recompute;
#[cfg(feature = "test-hooks")]
mod silent_clear;
mod subscription;

#[allow(unused_imports)] // 菜单的文档链接指着这里。
pub(crate) use diffs::send;
#[cfg(feature = "test-hooks")]
pub(crate) use silent_clear::mark_silent_clear;
pub(crate) use subscription::TimelineSource;
pub use subscription::TimelineSubscription;

#[cfg(test)]
#[path = "room_lifecycle_tests.rs"]
mod room_lifecycle_tests;

#[cfg(test)]
#[path = "timeline_tests.rs"]
mod tests;
