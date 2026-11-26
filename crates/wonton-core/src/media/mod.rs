//! 媒体与传输（ADR-0113）在服务器核的存储层与状态机。
//!
//! [`store`] 是任务、播放会话、队列与配额账本的唯一落点；[`task_service`] 是任务状态机，
//! [`session_service`] 管播放会话的状态与接管。HTTP、事件和提供方客户端不在这里。

pub mod session_service;
pub mod store;
pub mod task_service;

#[cfg(test)]
mod tests;

pub use store::{Ledger, MediaStoreError, Store};
pub use task_service::{TaskService, TaskServiceError};
