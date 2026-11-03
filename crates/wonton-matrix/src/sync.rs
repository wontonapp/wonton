//! 同步：Simplified Sliding Sync（MSC4186），由 matrix-sdk-ui 的 `SyncService` 驱动。
//!
//! `SyncService` 同时跑房间列表和端到端加密两条 Sliding Sync 连接，和 Element X 的用法一致。
//! 不开它的离线模式：离线模式把所有同步错误（包括设备被吊销）都当成离线并在后台重试，会掩盖吊销。出错后的重启由
//! [`MatrixSession::keep_syncing`] 做，它分得清吊销。

use std::time::{Duration, Instant};

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
    /// 同步因错误停止，要再次 [`MatrixSession::start_sync`]，[`MatrixSession::keep_syncing`] 会自动做。设备被吊销时
    /// 是 [`Error::Unauthorized`]，只能重新配对。
    Failed(Error),
}

/// 同步出错后第一次重启前等多久。
const FIRST_RESTART: Duration = Duration::from_secs(2);
/// 间隔最长这么久，连接恢复后最晚这么久接着同步。
const MAX_RESTART: Duration = Duration::from_secs(30);

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

    /// 同步因错误停下就重启，连续出错时间隔加倍，直到被中止。在同一个会话上重启：打开着的房间继续收到更新，不用
    /// 重新登录、重新打开。服务器拒绝身份（设备被吊销）时重启也没用，到此结束。
    pub async fn keep_syncing(&self) {
        let mut statuses = std::pin::pin!(self.sync_status());
        let mut delay = FIRST_RESTART;
        let mut restarted = Instant::now();
        while let Some(status) = statuses.next().await {
            let SyncStatus::Failed(error) = status else {
                continue;
            };
            if matches!(error, Error::Unauthorized { .. }) {
                return;
            }
            // 上次重启以后跑了一阵才出错，算恢复过，间隔从头算。重启本身成功不算数：服务器不通时它也可能先成功。
            if restarted.elapsed() > MAX_RESTART {
                delay = FIRST_RESTART;
            }
            tracing::info!(?delay, "同步停了，稍后重启：{error}");
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = self.resume_sync.notified() => {}
            }
            delay = (delay * 2).min(MAX_RESTART);
            restarted = Instant::now();
            // 失败时状态流里也有，下一轮处理。
            self.start_sync().await.ok();
        }
    }

    /// 调用方知道网络恢复了（隧道重新连上、App 回到前台）：发送队列马上全部重新打开，停着的同步马上重启，都不等
    /// 退避的间隔。
    pub fn resume(&self) {
        self.resume_sending.notify_one();
        self.resume_sync.notify_one();
    }

    /// 同步状态流：第一项是当前状态，之后每次变化给出新状态。
    pub fn sync_status(&self) -> impl Stream<Item = SyncStatus> + use<> {
        let subscriber = self.sync.state();
        let current = SyncStatus::from(&subscriber.get());
        stream::once(future::ready(current)).chain(subscriber.map(|state| SyncStatus::from(&state)))
    }
}
