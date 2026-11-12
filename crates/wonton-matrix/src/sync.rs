//! 同步：Simplified Sliding Sync（MSC4186），由 matrix-sdk-ui 的 `SyncService` 驱动。
//!
//! `SyncService` 同时跑房间列表和端到端加密两条 Sliding Sync 连接，和 Element X 的用法一致。
//! 不开它的离线模式：离线模式把所有同步错误（包括设备被吊销）都当成离线并在后台重试，会掩盖吊销。出错后的重启由
//! [`MatrixSession::keep_syncing`] 做，它分得清吊销。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk_ui::{
    room_list_service::{self, SyncIndicator},
    sync_service,
};

use crate::{Error, MatrixSession, Result};

/// 同步状态，供 ViewModel 展示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    /// 未启动或已停止。
    Idle,
    /// 已启动，房间列表第一次同步还没完成：这时没有房间不代表真的没有。
    Starting,
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
/// 追赶比这短就不提示。
const CATCH_UP_NOTICE_DELAY: Duration = Duration::from_millis(500);
/// 起步等这么久就报一次停在哪个状态。
const WAIT_REPORT: Duration = Duration::from_secs(5);

/// 各设备最近一次起步等待的现场，见 [`last_sync_diagnostic`]。一个进程里有多台设备（测试并行跑）时按设备号分开，
/// 免得把别人的等待当成自己的。
static WAIT_DIAGNOSTIC: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

/// `device_id` 上一次 [`MatrixSession::start_sync`] 报的等待现场，空串是还没报过。日志跟着 App 的日志开关走，
/// 这个不打开日志也拿得到，超时的时候能直接写进失败说明。
pub fn last_sync_diagnostic(device_id: &str) -> String {
    WAIT_DIAGNOSTIC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(device_id)
        .cloned()
        .unwrap_or_default()
}

fn record_sync_diagnostic(device_id: &str, text: &str) {
    WAIT_DIAGNOSTIC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(device_id.to_owned(), text.to_owned());
}

/// 等待现场的说明：等多久了、同步服务和房间列表各在哪个状态。房间列表要等一个应答回来才离开 `Init`，所以 `Init`
/// 是第一个应答还没回来（请求没发出去或还在路上都算），`SettingUp` 是回来过一批、还没跟上。
fn format_wait(
    elapsed: Duration,
    service: &sync_service::State,
    list: &room_list_service::State,
) -> String {
    format!("同步还停在起步：等了 {elapsed:?}，服务 {service:?}，房间列表 {list:?}")
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

/// 房间列表至少同步完成过一次。SDK 启动同步服务时马上报 `Running`，不等第一个响应，所以要看房间列表自己的状态。
/// `SettingUp` 只收到了第一批房间，和 [`MatrixSession::start_sync`] 一样等到 `Running`。
fn has_synced(list: &room_list_service::State) -> bool {
    match list {
        room_list_service::State::Init | room_list_service::State::SettingUp => false,
        room_list_service::State::Recovering | room_list_service::State::Running => true,
        room_list_service::State::Error { from }
        | room_list_service::State::Terminated { from } => has_synced(from),
    }
}

fn status_of(service: &sync_service::State, list: &room_list_service::State) -> SyncStatus {
    match SyncStatus::from(service) {
        SyncStatus::Running if !has_synced(list) => SyncStatus::Starting,
        status => status,
    }
}

impl MatrixSession {
    /// 启动同步，等到房间列表首次同步完成才返回。
    ///
    /// 起步可能久（服务器慢、请求在重试），期间每 [`WAIT_REPORT`] 把停在哪个状态打发出来、记进
    /// [`last_sync_diagnostic`]。没有这些，卡住时只剩一个 `Starting`，看不出卡在哪一步。
    pub async fn start_sync(&self) -> Result<()> {
        let mut list_state = self.sync.room_list_service().state();
        let mut service_state = self.sync.state();
        let started = Instant::now();
        {
            // SyncService 的 Error 在两条同步任务及过期清理结束后才发布。观察房间列表的 Error
            // 异步补挂会与重新启动竞跑；持订阅锁把恢复名单和启动串起来，健康服务不重发。
            let mut subscriptions = self.subscriptions.lock().await;
            if !matches!(service_state.get(), sync_service::State::Running) {
                self.resubscribe(&mut subscriptions).await;
                self.sync.start().await;
            }
        }
        let mut report =
            tokio::time::interval_at((Instant::now() + WAIT_REPORT).into(), WAIT_REPORT);
        report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            match SyncStatus::from(&service_state.get()) {
                SyncStatus::Failed(error) => return Err(error),
                // 启动之后同步服务又停了（被 `stop_sync` 或终结）：再等也不会有应答。
                SyncStatus::Idle => return Err(Error::Upstream("同步服务已停止".to_owned())),
                SyncStatus::Starting | SyncStatus::Running => {}
            }
            if list_state.get() == room_list_service::State::Running {
                return Ok(());
            }
            let changed = tokio::select! {
                state = service_state.next() => state.is_some(),
                state = list_state.next() => state.is_some(),
                _ = report.tick() => {
                    let wait = format_wait(started.elapsed(), &service_state.get(), &list_state.get());
                    tracing::warn!("{wait}");
                    record_sync_diagnostic(self.device_id().as_str(), &wait);
                    true
                }
            };
            if !changed {
                return Err(Error::Upstream("同步服务已关闭".to_owned()));
            }
        }
    }

    /// 停止同步（App 进入后台时调用）。
    pub async fn stop_sync(&self) {
        let _subscriptions = self.subscriptions.lock().await;
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
            // 同步被 403 拒绝也一样：每次重启都会被拒。
            if matches!(error, Error::Unauthorized { .. } | Error::Forbidden { .. }) {
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
            // 失败也在状态流里，下一轮处理；start_sync 会先恢复订阅再启动。
            self.start_sync().await.ok();
        }
    }

    /// 调用方知道网络恢复了（隧道重新连上、App 回到前台）：发送队列马上全部重新打开，停着的同步马上重启，都不等
    /// 退避的间隔。
    pub fn resume(&self) {
        self.resume_sending.notify_one();
        self.resume_sync.notify_one();
    }

    /// 同步是否在追赶：启动或出错之后还没收到第一个响应。
    pub fn catching_up(&self) -> impl Stream<Item = bool> + use<> {
        self.sync
            .room_list_service()
            .sync_indicator(CATCH_UP_NOTICE_DELAY, Duration::ZERO)
            .map(|indicator| matches!(indicator, SyncIndicator::Show))
    }

    /// 同步状态流：第一项是当前状态，之后每次变化给出新状态。
    pub fn sync_status(&self) -> impl Stream<Item = SyncStatus> + use<> {
        let service = self.sync.state();
        let service_now = self.sync.state();
        let lists = self.sync.room_list_service();
        let current = status_of(&service.get(), &lists.state().get());
        let list_changes = lists.state().filter_map(move |list| {
            let service = service_now.get();
            // 其他状态只由同步服务报，同一次失败不重复：`keep_syncing` 每报一次失败就重启一次。
            future::ready(
                matches!(service, sync_service::State::Running).then(|| status_of(&service, &list)),
            )
        });
        let service_changes = service.map(move |service| status_of(&service, &lists.state().get()));
        stream::once(future::ready(current)).chain(stream::select(service_changes, list_changes))
    }
}
