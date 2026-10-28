//! 同步：Simplified Sliding Sync（MSC4186），由 matrix-sdk-ui 的 `SyncService` 驱动。
//!
//! `SyncService` 同时跑房间列表和端到端加密两条 Sliding Sync 连接，和 Element X 的用法一致。
//! 不开它的离线模式：离线模式把所有同步错误（包括设备被吊销）都当成离线并在后台重试，会掩盖吊销。

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk_ui::{room_list_service, sync_service};

use crate::{Error, MatrixSession, Result};

/// 同步状态，供 ViewModel 展示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    /// 未启动或已停止。
    Idle,
    /// 正在同步。
    Running,
    /// 同步因错误停止，需要再次 [`MatrixSession::start_sync`]。设备被吊销时是
    /// [`Error::Unauthorized`]，只能重新配对。
    Failed(Error),
}

impl From<&sync_service::State> for SyncStatus {
    fn from(state: &sync_service::State) -> Self {
        match state {
            sync_service::State::Idle | sync_service::State::Terminated => Self::Idle,
            sync_service::State::Running => Self::Running,
            sync_service::State::Error(error) => Self::Failed(error.as_ref().into()),
            // 只有开了离线模式才会进入这个状态，本封装不开。
            sync_service::State::Offline => {
                Self::Failed(Error::Transport("同步进入离线模式".to_owned()))
            }
        }
    }
}

impl MatrixSession {
    /// 启动同步，等到房间列表首次同步完成才返回。
    pub async fn start_sync(&self) -> Result<()> {
        let mut service_state = self.sync.state();
        let mut list_state = self.sync.room_list_service().state();
        self.sync.start().await;
        loop {
            if let SyncStatus::Failed(error) = SyncStatus::from(&service_state.get()) {
                return Err(error);
            }
            if list_state.get() == room_list_service::State::Running {
                return Ok(());
            }
            let changed = tokio::select! {
                state = service_state.next() => state.is_some(),
                state = list_state.next() => state.is_some(),
            };
            if !changed {
                return Err(Error::Upstream("同步服务已关闭".to_owned()));
            }
        }
    }

    /// 停止同步（App 进入后台时调用）。
    pub async fn stop_sync(&self) {
        self.sync.stop().await;
    }

    /// 同步状态流：第一项是当前状态，之后每次变化给出新状态。
    pub fn sync_status(&self) -> impl Stream<Item = SyncStatus> + use<> {
        let subscriber = self.sync.state();
        let current = SyncStatus::from(&subscriber.get());
        stream::once(future::ready(current)).chain(subscriber.map(|state| SyncStatus::from(&state)))
    }
}
