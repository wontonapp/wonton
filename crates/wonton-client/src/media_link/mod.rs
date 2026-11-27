//! 媒体与传输的客户端核（ADR-0113）：任务和播放会话的事实源在家里服务器的核心，这里只记本机这份、
//! 发这台设备的请求、把状态翻成视图交给壳。
//!
//! 每次连上都按服务器对齐（[`state::Local::align`]）。系统播放器和后台传输只连本机回环（[`loopback`]），
//! 起播带上壳报过的设备档（[`profile`]）。动作经 [`MediaLink::dispatch`] 进来：播放控制、停止、
//! 队列改动发给核心，回应落到本地再推给壳；没成的原因落在视图的 `failure` 上，壳按类别出字。上传、下载的
//! 字节由执行层负责，不在这里。

mod api;
mod download;
mod loopback;
#[cfg(test)]
mod loopback_tests;
mod player_bridge;
#[cfg(test)]
mod player_bridge_tests;
mod profile;
mod queue;
mod state;
mod subscription;
#[cfg(test)]
mod tests;
mod video;
mod video_play;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::time::Instant;
use wonton_protocol::media::api::{
    MEDIA_API_PREFIX, SessionList, TaskAction, TaskList, queue_path, session_control_path,
    session_path, session_progress_path, session_stop_path, sessions_path, task_action_path,
    tasks_path,
};
use wonton_protocol::media::provider::{ItemsPage, ItemsQuery, PlaybackStopped};
use wonton_protocol::media::{
    MediaAction, MediaFailure, MediaFailureCode, MediaTask, MediaViewModel, PlaybackRequest,
    PlaybackSession, PlaybackSessionId, PlaybackViewModel, Queue, TaskCreate, TaskState,
};
use wonton_protocol::{CoreError, HomeStatus, HomeViewModel};

use crate::home::HomeLink;
pub(crate) use api::{MediaApi, failure};
use download::Downloads;
pub use download::OfflineState;
use loopback::Loopback;
use player_bridge::{Bridge, Report};
pub use player_bridge::{PlayerCommand, PlayerEvent};
pub use profile::PlayerSupport;
use profile::Profiles;
use state::Local;
pub use subscription::{MediaSubscription, PlaybackSubscription};
pub use video::{
    VideoActionFailure, VideoActionResult, VideoDetail, VideoEntry, VideoEntryKind, VideoHome,
    VideoLibrary, VideoLibraryKind, VideoMediaInfo, VideoPerson, VideoRole, VideoShelf,
    VideoShelfKind, VideoSort, VideoWall, VideoWallQuery,
};
pub use video_play::VideoPlayback;
pub(crate) use video_play::session_id;

/// 壳的播放器该处在哪一步；只给壳用，不进协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PlayerPhase {
    Playing,
    Paused,
    /// 有会话、还没有可播的计划（起播或切轨后核心在重取）。
    Buffering,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PlayerState {
    pub phase: PlayerPhase,
    pub position_ms: u64,
}

pub(crate) struct MediaLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    state: Mutex<Local>,
    loopback: Loopback,
    profiles: Profiles,
    bridge: Mutex<Bridge>,
    downloads: Arc<Downloads>,
    reports: video_play::Reports,
    media: watch::Sender<MediaViewModel>,
    playback: watch::Sender<Option<PlaybackViewModel>>,
}

/// 两个视图的发送端；换服务器时接着用上一个作用域的，壳的订阅不用换。
#[derive(Clone)]
pub(crate) struct MediaViews {
    pub(crate) media: watch::Sender<MediaViewModel>,
    pub(crate) playback: watch::Sender<Option<PlaybackViewModel>>,
}

impl MediaLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        dir: &Path,
        views: Option<MediaViews>,
    ) -> Arc<Self> {
        let local = Local::default();
        let views = match views {
            Some(views) => {
                views.media.send_replace(local.media_view());
                views.playback.send_replace(None);
                views
            }
            None => MediaViews {
                media: watch::Sender::new(local.media_view()),
                playback: watch::Sender::new(None),
            },
        };
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.clone(),
            state: Mutex::new(local),
            loopback: Loopback::new(),
            profiles: Profiles::open(dir),
            bridge: Mutex::new(Bridge::default()),
            downloads: Downloads::open(dir),
            reports: video_play::Reports::default(),
            media: views.media,
            playback: views.playback,
        });
        runtime.spawn(follow_home(Arc::downgrade(&link), home.subscribe()));
        link
    }

    pub(crate) fn views(&self) -> MediaViews {
        MediaViews {
            media: self.media.clone(),
            playback: self.playback.clone(),
        }
    }

    pub(crate) fn player_state(&self) -> PlayerState {
        player_state(self.lock().session())
    }

    fn lock(&self) -> MutexGuard<'_, Local> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 改本地这份，再把两个视图都推一次。
    fn update<R>(&self, change: impl FnOnce(&mut Local) -> R) -> R {
        let mut local = self.lock();
        let result = change(&mut local);
        self.media.send_replace(local.media_view());
        self.playback.send_replace(local.playback_view());
        result
    }

    fn api(&self) -> Result<MediaApi, MediaFailure> {
        MediaApi::connect(&self.home)
    }

    /// 拉服务器上这个人的任务、会话和队列，换掉本地的。
    pub(crate) async fn sync(&self) -> Result<(), MediaFailure> {
        let pulled = async {
            let api = self.api()?;
            let tasks: TaskList = api.get(&tasks_path()).await?;
            let sessions: SessionList = api.get(&sessions_path()).await?;
            Ok::<_, MediaFailure>((api, tasks.tasks, sessions.sessions))
        };
        match pulled.await {
            Ok((api, tasks, sessions)) => {
                self.run_downloads(&api, &tasks);
                self.update(|local| local.align(tasks, sessions));
                // 队列单独拉：拉不到不挡任务和会话的对齐，原因记在播放视图上。壳改队列要带它的版本。
                match api.get::<Queue>(&queue_path()).await {
                    Ok(queue) => self.update(|local| local.put_queue(queue)),
                    Err(failure) => self.update(|local| local.playback_failed(failure)),
                }
                Ok(())
            }
            Err(failure) => {
                self.update(|local| local.tasks_failed(failure.clone()));
                Err(failure)
            }
        }
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: MediaAction) -> Result<(), CoreError> {
        match action {
            MediaAction::StartUpload { .. } => Err(CoreError::NotReady {
                reason: "media uploads belong to the upload worker, which this client core does not have yet".into(),
            }),
            // 核心让这台设备开始或接着下：重拉任务表，按它对齐。
            MediaAction::StartDownload { .. } => {
                let link = self.clone();
                self.runtime.spawn(async move {
                    if let Err(failure) = link.sync().await {
                        tracing::warn!(code = ?failure.code, error = %failure.message, "download tasks could not be aligned with the server");
                    }
                });
                Ok(())
            }
            // 新核心发的新动作，这一版跳过（AGENTS.md 的向前兼容）。
            MediaAction::Other(_) => Ok(()),
            action => {
                let link = self.clone();
                self.runtime.spawn(async move {
                    match link.send(action).await {
                        Ok(()) => link.update(Local::playback_succeeded),
                        Err(failure) => link.update(|local| local.playback_failed(failure)),
                    }
                });
                Ok(())
            }
        }
    }

    async fn send(&self, action: MediaAction) -> Result<(), MediaFailure> {
        let api = self.api()?;
        match action {
            MediaAction::Playback { session } => {
                let read: PlaybackSession = api.get(&session_path(session)).await?;
                self.update(|local| local.put_session(read));
            }
            MediaAction::Control { session, ctl } => {
                let body = MediaAction::Control { session, ctl };
                let read: PlaybackSession = api.post(&session_control_path(session), &body).await?;
                self.update(|local| local.put_session(read));
            }
            MediaAction::Stop { session, reason } => {
                let position_ms = self
                    .lock()
                    .session()
                    .filter(|s| s.id == session)
                    .map_or(0, |s| s.position_ms);
                let body = PlaybackStopped {
                    position_ms,
                    reason,
                };
                api.post_empty(&session_stop_path(session), &body).await?;
                let read: PlaybackSession = api.get(&session_path(session)).await?;
                self.update(|local| local.put_session(read));
            }
            MediaAction::Queue { revision, op, .. } => {
                let before = self.lock().queue().cloned();
                queue::change(&api, before.as_ref(), revision, op, |queue| {
                    self.update(|local| local.put_queue(queue));
                })
                .await?;
            }
            MediaAction::StartUpload { .. }
            | MediaAction::StartDownload { .. }
            | MediaAction::Other(_) => {}
        }
        Ok(())
    }

    /// 起播：核心向提供方要计划，回会话；`person`、`device` 留空由核心按这台设备填。
    pub(crate) async fn start_playback(
        &self,
        mut request: PlaybackRequest,
    ) -> Result<PlaybackViewModel, MediaFailure> {
        self.profiles.fill(&mut request)?;
        let session: PlaybackSession = self.api()?.post(&sessions_path(), &request).await?;
        self.update(|local| {
            local.put_session(session);
            local.playback_view()
        })
        .ok_or_else(|| failure(MediaFailureCode::Protocol, "起播后没有会话"))
    }

    pub(crate) async fn create_task(&self, create: TaskCreate) -> Result<MediaTask, MediaFailure> {
        let api = self.api()?;
        let task: MediaTask = api.post(&tasks_path(), &create).await?;
        self.run_downloads(&api, std::slice::from_ref(&task));
        self.update(|local| local.put_task(task.clone()));
        Ok(task)
    }

    /// 这台设备的下载任务交给执行层：该下的下，停了的停。
    fn run_downloads(&self, api: &MediaApi, tasks: &[MediaTask]) {
        let device = self.home.view().device_id;
        self.downloads.reconcile(&self.runtime, api, &device, tasks);
    }

    /// 这一条在这台设备上的离线副本。
    pub(crate) fn offline(&self, item_id: &str) -> Option<OfflineState> {
        self.downloads.offline(item_id)
    }

    /// 删掉这一条的离线副本。
    pub(crate) fn remove_offline(&self, item_id: &str) -> Result<(), MediaFailure> {
        self.downloads.remove(item_id)
    }

    /// 核心说失败了的下载再来一次：核心开新任务，旧任务的半截归它，这台设备接着下。
    pub(crate) async fn retry_download(&self, item_id: &str) -> Result<MediaTask, MediaFailure> {
        let old = self.downloads.failed_task(item_id).ok_or_else(|| {
            failure(
                MediaFailureCode::Protocol,
                crate::l10n::text("video-download-not-failed", &[]),
            )
        })?;
        let api = self.api()?;
        let task: MediaTask = api
            .post(
                &task_action_path(old, TaskAction::Retry),
                &serde_json::json!({}),
            )
            .await?;
        self.downloads.adopt(old, task.id)?;
        self.run_downloads(&api, std::slice::from_ref(&task));
        self.update(|local| local.put_task(task.clone()));
        Ok(task)
    }

    /// 本机回环前缀：播放器、后台传输连 `{前缀}/sessions/{id}/stream` 这样的地址。
    pub(crate) async fn loopback(&self) -> Result<String, MediaFailure> {
        let home = self.home.clone();
        self.loopback
            .prefix(Arc::new(move || MediaApi::connect(&home)))
            .await
    }

    /// 壳报的播放器能力，之后起播都带上。
    pub(crate) fn report_player_support(
        &self,
        support: PlayerSupport,
    ) -> Result<wonton_protocol::media::DeviceProfile, MediaFailure> {
        self.profiles.report(support)
    }

    /// 按种类列提供方的条目（原生的浏览页用）。
    pub(crate) async fn items(&self, query: ItemsQuery) -> Result<ItemsPage, MediaFailure> {
        self.api()?
            .get(&format!("{MEDIA_API_PREFIX}{}", query.path()))
            .await
    }
}

impl MediaLink {
    /// 壳的播放器报的事（M20）：本机的位置先跟上，再按节奏把位置、换曲或停止发给核心；没成的原因落在
    /// 播放视图上。回给壳这次要它做的（多数时候什么也不用做）。
    pub(crate) fn report_player(self: &Arc<Self>, event: PlayerEvent) -> PlayerCommand {
        let wall_clock = unix_ms();
        let decided = self.update(|local| {
            let session = local.session()?.clone();
            local.set_position(session.id, event.position_ms());
            if let PlayerEvent::Failed { code, message, .. } = &event {
                local.playback_failed(failure(code.clone(), message.clone()));
            }
            let decision = self
                .bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .decide(&session, local.queue(), &event, Instant::now(), wall_clock);
            Some((session.id, decision))
        });
        let Some((session, decision)) = decided else {
            return PlayerCommand::Keep;
        };
        if let Some(report) = decision.report {
            let link = self.clone();
            self.runtime.spawn(async move {
                if let Err(failure) = link.send_report(session, report).await {
                    link.update(|local| local.playback_failed(failure));
                }
            });
        }
        decision.command
    }

    async fn send_report(
        &self,
        session: PlaybackSessionId,
        report: Report,
    ) -> Result<(), MediaFailure> {
        let api = self.api()?;
        match report {
            Report::Progress(body) => api.post_empty(&session_progress_path(session), &body).await,
            Report::Control(ctl) => {
                let body = MediaAction::Control { session, ctl };
                let read: PlaybackSession = api.post(&session_control_path(session), &body).await?;
                self.update(|local| local.put_session(read));
                // 换曲挪了队列的指针：取回来，壳的队列页跟着走。
                let queue: Queue = api.get(&queue_path()).await?;
                self.update(|local| local.put_queue(queue));
                Ok(())
            }
            Report::Stop(body) => {
                api.post_empty(&session_stop_path(session), &body).await?;
                let read: PlaybackSession = api.get(&session_path(session)).await?;
                self.update(|local| local.put_session(read));
                Ok(())
            }
        }
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// 会话翻成播放器该在的状态。
fn player_state(session: Option<&PlaybackSession>) -> PlayerState {
    let Some(session) = session else {
        return PlayerState {
            phase: PlayerPhase::Stopped,
            position_ms: 0,
        };
    };
    // 暂停先于缓冲：暂停中换曲、重取计划途中也是暂停，不该显示转圈。
    let phase = if session.stop_reason.is_some() || session.state.is_terminal() {
        PlayerPhase::Stopped
    } else if matches!(
        session.state,
        TaskState::Paused(_) | TaskState::WaitingNetwork | TaskState::WaitingPower
    ) {
        PlayerPhase::Paused
    } else if session.plan.is_none() {
        PlayerPhase::Buffering
    } else {
        PlayerPhase::Playing
    };
    PlayerState {
        phase,
        position_ms: session.position_ms,
    }
}

/// 每次连上就和服务器对齐一次：断线期间本地的偏离由服务器的值盖掉。
async fn follow_home(link: Weak<MediaLink>, mut home: watch::Receiver<HomeViewModel>) {
    let mut was_connected = false;
    loop {
        let connected = home.borrow_and_update().status == HomeStatus::Connected;
        if connected && !was_connected {
            let Some(link) = link.upgrade() else {
                return;
            };
            if let Err(failure) = link.sync().await {
                tracing::warn!(code = ?failure.code, error = %failure.message, "media tasks failed to sync with the server");
            }
        }
        was_connected = connected;
        if home.changed().await.is_err() || link.strong_count() == 0 {
            return;
        }
    }
}
