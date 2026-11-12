//! “我来操作”（ADR-0024）：开始后隔一会儿续一次约。续约和交还在同一个任务里按顺序发，交还不会被还在路上的续约盖掉。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_protocol::{PluginFailure, PluginProblem, PluginTakeover, TakeoverState};

use super::{PluginsLink, lock};

const RENEW_EVERY: Duration = Duration::from_secs(10);
/// 和核心的租约一样长：过了这么久没续上，核心那边已经结束了，这边不能还说在操作。
const LEASE: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Default)]
pub(super) struct Slot {
    /// 每次开始或结束加一：之前的任务不再能写 `view.takeover`。
    epoch: u64,
    stop: Option<watch::Sender<bool>>,
    /// 交还之后留着：下一次开始要等它的 `DELETE` 发完，否则可能把新的结束掉。
    task: Option<JoinHandle<()>>,
}

impl PluginsLink {
    pub(crate) fn start_takeover(self: &Arc<Self>, id: String) {
        let mut slot = lock(&self.takeover);
        let going = self.view.borrow().takeover.as_ref().is_some_and(|current| {
            current.id == id
                && matches!(
                    current.state,
                    TakeoverState::Starting | TakeoverState::Operating
                )
        });
        if going {
            return;
        }
        slot.epoch += 1;
        let (stop, stopped) = watch::channel(false);
        if let Some(old) = slot.stop.replace(stop) {
            old.send_replace(true);
        }
        self.view.send_modify(|view| {
            view.takeover = Some(PluginTakeover {
                id: id.clone(),
                state: TakeoverState::Starting,
            });
        });
        let run = Run {
            link: Arc::downgrade(self),
            id,
            epoch: slot.epoch,
        };
        let previous = slot.task.take();
        slot.task = Some(self.runtime.spawn(run.hold(previous, stopped)));
    }

    pub(crate) fn end_takeover(&self, id: &str) {
        let mut slot = lock(&self.takeover);
        let current = self
            .view
            .borrow()
            .takeover
            .as_ref()
            .is_some_and(|current| current.id == id);
        if !current {
            return;
        }
        slot.epoch += 1;
        if let Some(stop) = slot.stop.take() {
            stop.send_replace(true);
        }
        self.view.send_modify(|view| view.takeover = None);
    }

    /// 进后台算交还。已经断了的留着，回来时主人还要看到。
    pub(crate) fn backgrounded(&self) {
        self.foreground.store(false, Ordering::SeqCst);
        self.poll_changed.notify_one();
        let going = self
            .view
            .borrow()
            .takeover
            .as_ref()
            .filter(|current| {
                matches!(
                    current.state,
                    TakeoverState::Starting | TakeoverState::Operating
                )
            })
            .map(|current| current.id.clone());
        if let Some(id) = going {
            self.end_takeover(&id);
        }
    }

    /// 换了服务器：上一台等租约自己过期，什么都不再发。
    pub(super) fn forget_takeover(&self) {
        let mut slot = lock(&self.takeover);
        slot.epoch += 1;
        slot.stop = None;
        if let Some(task) = slot.task.take() {
            task.abort();
        }
        self.view.send_modify(|view| view.takeover = None);
    }

    fn settle_takeover(&self, epoch: u64, state: TakeoverState) {
        let slot = lock(&self.takeover);
        if slot.epoch != epoch {
            return;
        }
        self.view.send_if_modified(|view| match &mut view.takeover {
            Some(current) if current.state != state => {
                current.state = state;
                true
            }
            _ => false,
        });
    }
}

struct Run {
    link: Weak<PluginsLink>,
    id: String,
    epoch: u64,
}

impl Run {
    async fn hold(self, previous: Option<JoinHandle<()>>, mut stopped: watch::Receiver<bool>) {
        if let Some(previous) = previous {
            let _ = previous.await;
        }
        let mut renewed: Option<Instant> = None;
        while !*stopped.borrow() {
            let Some(link) = self.link.upgrade() else {
                return;
            };
            let timeout = renewed.map_or(REQUEST_TIMEOUT, |at| {
                REQUEST_TIMEOUT.min(LEASE.saturating_sub(at.elapsed()))
            });
            match self.renew(&link, timeout).await {
                Ok(()) => {
                    renewed = Some(Instant::now());
                    link.settle_takeover(self.epoch, TakeoverState::Operating);
                }
                Err(problem) => {
                    let Some(at) = renewed else {
                        link.settle_takeover(self.epoch, TakeoverState::NotStarted { problem });
                        return;
                    };
                    // 只有连不上才值得再试一次，核心明确拒绝（插件停了、设备被吊销）的再试也一样。
                    let retry = problem.reason == PluginFailure::TransportUnavailable
                        && at.elapsed() + RENEW_EVERY < LEASE;
                    if !retry {
                        link.settle_takeover(self.epoch, TakeoverState::Lost { problem });
                        return;
                    }
                }
            }
            drop(link);
            tokio::select! {
                _ = stopped.changed() => {}
                () = tokio::time::sleep(RENEW_EVERY) => {}
            }
        }
        if renewed.is_some()
            && let Some(link) = self.link.upgrade()
            && let Err(problem) = self.hand_back(&link).await
        {
            // 主人已经走了，没有地方可说；核心的租约会自己过期。
            tracing::info!("交还没有送到：{}", problem.message);
        }
    }

    async fn renew(&self, link: &PluginsLink, timeout: Duration) -> Result<(), PluginProblem> {
        link.api()?
            .put_unit(&["plugins", &self.id, "takeover"], timeout)
            .await
    }

    async fn hand_back(&self, link: &PluginsLink) -> Result<(), PluginProblem> {
        link.api()?
            .delete(&["plugins", &self.id, "takeover"], &[], REQUEST_TIMEOUT)
            .await
    }
}
