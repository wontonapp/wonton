//! 媒体与传输（ADR-0113）在服务器核的存储层与状态机。
//!
//! [`store`] 是任务、播放会话、队列与配额账本的唯一落点；[`task_service`] 是任务状态机，字节在走时
//! 任务怎么变在 `transfer`；[`session_service`] 管播放会话的状态与接管。[`provider`] 选提供方、按人签
//! 凭据。HTTP 在 `gateway::media`，事件不在这里。

use std::path::Path;
use std::sync::Arc;

mod leave;
pub(crate) mod prefetch;
pub(crate) mod provider;
pub mod session_service;
pub mod store;
pub mod task_service;
mod transfer;

#[cfg(test)]
mod offline_tests;
#[cfg(test)]
mod provider_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub(crate) use leave::LeaveError;
pub use session_service::SessionService;
pub use store::{Ledger, MediaStoreError, Store};
pub use task_service::{TaskService, TaskServiceError};

use crate::http::HttpClient;
use crate::plugin::PluginManager;
use provider::{MediaProviders, ProviderDirectory};

/// 网关用的媒体服务：一个库、两台状态机和提供方目录。
pub struct Media {
    pub store: Arc<Store>,
    pub tasks: TaskService,
    pub sessions: SessionService,
    pub(crate) providers: MediaProviders,
    pub(crate) queued: prefetch::QueuedStreams,
}

impl Media {
    /// 打开 `dir` 下的媒体库；按人签的提供方凭据放在 `dir/keys`。
    pub fn open(
        dir: &Path,
        plugins: Arc<PluginManager>,
        http: HttpClient,
    ) -> Result<Self, MediaStoreError> {
        Self::with_directory(dir, plugins, http)
    }

    pub(crate) fn with_directory(
        dir: &Path,
        directory: Arc<dyn ProviderDirectory>,
        http: HttpClient,
    ) -> Result<Self, MediaStoreError> {
        let store = Arc::new(Store::open(dir)?);
        Ok(Self {
            tasks: TaskService::new(store.clone()),
            sessions: SessionService::new(store.clone()),
            providers: MediaProviders::new(directory, http, dir.join("keys")),
            queued: prefetch::QueuedStreams::default(),
            store,
        })
    }
}
