//! 播放桥（ADR-0113 M20）：壳的播放器只报发生了什么，什么时候、往核心报什么由这里定。
//!
//! 播放中每 5 秒报一次位置，暂停和拖动各报一次；一项放完时队列里还有下一项就换下一项，没有就报停止
//! （`finished`）；壳停了声报停止（`user`）。会话已经被别处停了（另一台设备接管、Agent 停、退出家庭）
//! 时，壳停声后报的最后位置按会话已有的停止原因再报一次停止：核心对重复的停止只把位置往后推，这就是
//! 下次续播的断点。节流放在客户端核：壳报得再勤也不会打到核心。

use std::time::Duration;

use tokio::time::Instant;
use wonton_protocol::media::provider::{PlaybackProgress, PlaybackStopped};
use wonton_protocol::media::{
    ControlOp, MediaFailureCode, PlaybackSession, PlaybackSessionId, Queue, QueueMode, StopReason,
    TaskState, Timestamp,
};

/// 播放中两次位置上报至少隔这么久（ADR-0113 第 4 节）。
pub(crate) const PROGRESS_EVERY: Duration = Duration::from_secs(5);

/// 壳的播放器报的事；位置都是这一项里的毫秒。
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PlayerEvent {
    /// 正在出声。播放器的时钟每走一段就报，报得再勤也只按节奏发给核心。
    Playing {
        position_ms: u64,
    },
    Paused {
        position_ms: u64,
    },
    /// 播放器跳到了新位置（用户拖了进度条，或者系统的锁屏控件拖的）。
    Seeked {
        position_ms: u64,
    },
    /// 这一项放到头了。
    Finished {
        position_ms: u64,
    },
    /// 这台设备停了声：用户点了停，或者会话已被别处停了、壳跟着停。
    Stopped {
        position_ms: u64,
    },
    /// 播放器放不下去。`code` 由壳按原因归类（连不上回环、格式放不了、条目没了），原话进 `message`。
    Failed {
        position_ms: u64,
        code: MediaFailureCode,
        message: String,
    },
}

impl PlayerEvent {
    pub(crate) fn position_ms(&self) -> u64 {
        match self {
            Self::Playing { position_ms }
            | Self::Paused { position_ms }
            | Self::Seeked { position_ms }
            | Self::Finished { position_ms }
            | Self::Stopped { position_ms }
            | Self::Failed { position_ms, .. } => *position_ms,
        }
    }
}

/// 壳收到事件后要做的；多数时候不用做什么，播放器跟着播放视图走。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PlayerCommand {
    Keep,
    /// 跳到这个位置接着放（单曲循环放完了回到开头）。
    SeekTo {
        position_ms: u64,
    },
}

/// 这次事件要发给核心的请求。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Report {
    Progress(PlaybackProgress),
    Control(ControlOp),
    Stop(PlaybackStopped),
}

/// 一次事件的结果：发什么、壳做什么。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Decision {
    pub(crate) report: Option<Report>,
    pub(crate) command: PlayerCommand,
}

impl Decision {
    fn report(report: Report) -> Self {
        Self {
            report: Some(report),
            command: PlayerCommand::Keep,
        }
    }

    fn nothing() -> Self {
        Self {
            report: None,
            command: PlayerCommand::Keep,
        }
    }
}

/// 记着上一次给哪条会话报过什么，用来节流和不重复报停止。
#[derive(Default)]
pub(crate) struct Bridge {
    last_progress: Option<(PlaybackSessionId, Instant)>,
    /// 已经替它报过最后位置的、别处停掉的会话。
    closed: Option<PlaybackSessionId>,
}

impl Bridge {
    pub(crate) fn decide(
        &mut self,
        session: &PlaybackSession,
        queue: Option<&Queue>,
        event: &PlayerEvent,
        now: Instant,
        wall_clock: Timestamp,
    ) -> Decision {
        let position_ms = event.position_ms();
        if let Some(reason) = stopped_because(session) {
            return self.after_stop(session.id, &reason, event, position_ms);
        }
        let progress = || {
            Report::Progress(PlaybackProgress {
                position_ms,
                wall_clock,
            })
        };
        let stop = |reason| {
            Report::Stop(PlaybackStopped {
                position_ms,
                reason,
            })
        };
        match event {
            PlayerEvent::Playing { .. } => {
                let due = self.last_progress.is_none_or(|(id, at)| {
                    id != session.id || now.duration_since(at) >= PROGRESS_EVERY
                });
                if !due {
                    return Decision::nothing();
                }
                self.last_progress = Some((session.id, now));
                Decision::report(progress())
            }
            PlayerEvent::Paused { .. } | PlayerEvent::Seeked { .. } => {
                self.last_progress = Some((session.id, now));
                Decision::report(progress())
            }
            PlayerEvent::Finished { .. } => finished(queue, stop(StopReason::Finished)),
            PlayerEvent::Stopped { .. } => Decision::report(stop(StopReason::User)),
            PlayerEvent::Failed { .. } => Decision::report(stop(StopReason::Error)),
        }
    }

    /// 会话已经停了：壳停声时报的位置按原来的停止原因补一次，之后的都不报。
    fn after_stop(
        &mut self,
        session: PlaybackSessionId,
        reason: &StopReason,
        event: &PlayerEvent,
        position_ms: u64,
    ) -> Decision {
        if matches!(event, PlayerEvent::Playing { .. }) || self.closed == Some(session) {
            return Decision::nothing();
        }
        self.closed = Some(session);
        Decision::report(Report::Stop(PlaybackStopped {
            position_ms,
            reason: reason.clone(),
        }))
    }
}

/// 会话停了没有、为什么停。停了的会话按协议都带着原因；没带原因却已经是终态的（旧核心、别家实现），按终态
/// 推一个原因，照样补报最后位置，不把断点丢掉。核心对已停的会话只把位置往后推，推的原因不会改掉什么。
fn stopped_because(session: &PlaybackSession) -> Option<StopReason> {
    if let Some(reason) = &session.stop_reason {
        return Some(reason.clone());
    }
    if !session.state.is_terminal() {
        return None;
    }
    Some(match session.state {
        TaskState::Done | TaskState::Partial => StopReason::Finished,
        TaskState::Failed(_) => StopReason::Error,
        _ => StopReason::User,
    })
}

/// 一项放完：单曲循环回到开头；队列里后面还有（或整列循环）就换下一项，由核心换会话的条目、重取计划；
/// 没有了才算整个放完。
fn finished(queue: Option<&Queue>, stop: Report) -> Decision {
    let Some(queue) = queue.filter(|queue| !queue.entries.is_empty()) else {
        return Decision::report(stop);
    };
    match queue.mode {
        QueueMode::RepeatOne => Decision {
            report: Some(Report::Control(ControlOp::SeekMs(0))),
            command: PlayerCommand::SeekTo { position_ms: 0 },
        },
        QueueMode::RepeatAll => Decision::report(Report::Control(ControlOp::Next)),
        _ => {
            let has_next = queue
                .cursor
                .is_some_and(|cursor| cursor + 1 < queue.entries.len());
            if has_next {
                Decision::report(Report::Control(ControlOp::Next))
            } else {
                Decision::report(stop)
            }
        }
    }
}
