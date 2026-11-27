//! 媒体与传输（ADR-0113）在服务器核的存储层与状态机。
//!
//! [`store`] 是任务、播放会话、队列与配额账本的唯一落点；[`task_service`] 是任务状态机，
//! [`session_service`] 管播放会话的状态与接管。[`provider`] 选提供方、按人签凭据；HTTP 和事件不在这里。

// 媒体接口（ADR-0113 M46）接上之前没有非测试调用点。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod provider;
pub mod session_service;
pub mod store;
pub mod task_service;

#[cfg(test)]
mod provider_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub use store::{Ledger, MediaStoreError, Store};
pub use task_service::{TaskService, TaskServiceError};
