//! 媒体与传输（ADR-0113）在服务器核的存储层。
//!
//! [`store`] 是任务、播放会话、队列与配额账本的唯一落点；[`task_service`] 是任务状态机。HTTP 和提供方客户端不在这里。

pub mod store;
pub mod task_service;

#[cfg(test)]
mod tests;

pub use store::{Ledger, MediaStoreError, Store};
pub use task_service::{TaskService, TaskServiceError};
