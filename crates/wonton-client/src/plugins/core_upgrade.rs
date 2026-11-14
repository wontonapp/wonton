//! 核心一键升级的状态机（ADR-0025 的“App 显示”）：结论只从核心的报告和断开前它给的时限推出来，壳只渲染。
//! 最后看到的“升级中”和主人跳过的版本存在本机，App 被杀后重开也接得上。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Method;
use serde::{Deserialize, Serialize};
use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, CoreUpgradeCheck, CoreUpgradeCheckRequest, CoreUpgradeCompat,
    CoreUpgradeCompatCategory, CoreUpgradeCompatDetail, CoreUpgradeFailure, CoreUpgradeOffer,
    CoreUpgradePhase, CoreUpgradeProblem, CoreUpgradeRequest, CoreUpgradeRun, CoreUpgradeRunState,
    CoreUpgradeStage, CoreUpgradeStatus, CoreUpgradeViewModel, PluginAction,
};

use super::api::{READ_TIMEOUT, SOURCE_TIMEOUT};
use super::{POLL_INTERVAL, PluginsLink, lock};
use crate::l10n::{CATALOG, language};

const PATH: [&str; 2] = ["core", "upgrade"];
const CHECK_PATH: [&str; 3] = ["core", "upgrade", "check"];
const SAVED_FILE: &str = "core-upgrade.json";
/// 查新版本在核心的后台进行，`POST` 的回应里还是上次的结果。查到新版本时状态会变成下载中，接着盯；没变化时最多再取这么多次。
const CHECK_FOLLOW_UPS: usize = 4;

/// 存在本机的部分。读不懂时当作没有：这台设备就和没看到过“升级中”的设备一样，只显示普通的连不上。
#[derive(Serialize, Deserialize)]
struct Saved {
    /// 这些记录属于哪台服务器。
    server: String,
    watching: Option<Watching>,
    skipped: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Watching {
    status: CoreUpgradeStatus,
    /// 用墙上时钟：单调时钟在 App 重开后不算数，要的又正是被杀掉期间过去了多久。
    deadline_ms: u64,
}

#[derive(Default)]
pub(super) struct UpgradeLocal {
    status: Option<CoreUpgradeStatus>,
    /// 最后一次看到“升级中”时，核心给的时限到什么时候。
    deadline_ms: u64,
    /// 最后一次取状态失败了。
    unreachable: bool,
    requesting: bool,
    /// 已经为这次升级发过取消。
    cancel_sent: Option<String>,
    skipped: Option<String>,
    problem: Option<CoreUpgradeProblem>,
    /// 还要再取几次，等后台的检查出结果。
    follow_ups: usize,
    /// 这轮检查是主人点的，要转圈。
    manual: bool,
    polling: bool,
    /// 发起、取消和换服务器时加一：之前发出的请求，结果不再算数。
    epoch: u64,
}

impl UpgradeLocal {
    /// 服务器最近报的运行版本；没取到过时为空。
    pub(super) fn running_version(&self) -> Option<String> {
        self.status
            .as_ref()
            .and_then(|status| status.current.version.clone())
    }

    fn running_run(&self) -> Option<&CoreUpgradeRun> {
        self.status
            .as_ref()
            .and_then(|status| status.run.as_ref())
            .filter(|run| matches!(run.state, CoreUpgradeRunState::InProgress { .. }))
    }

    /// 还要接着取状态。过了时限就停：请求会触发重连，不能一直连下去。之后打开升级页、回到前台或者隧道连上时再取。
    fn live(&self, now_ms: u64) -> bool {
        if self.follow_ups > 0 {
            return true;
        }
        if self.running_run().is_some() {
            return !self.unreachable || now_ms < self.deadline_ms;
        }
        let checking = self.status.as_ref().is_some_and(|status| {
            matches!(
                status.check,
                CoreUpgradeCheck::NotChecked
                    | CoreUpgradeCheck::Checking
                    | CoreUpgradeCheck::Downloading { .. }
            )
        });
        checking && !self.unreachable
    }

    fn view(&self, now_ms: u64) -> CoreUpgradeViewModel {
        let offer = match self.status.as_ref().map(|status| &status.check) {
            Some(CoreUpgradeCheck::Ready { offer })
                if !self.requesting && self.running_run().is_none() =>
            {
                Some(offer.clone())
            }
            _ => None,
        };
        let compat_details = offer.as_ref().map(compat_details).unwrap_or_default();
        CoreUpgradeViewModel {
            status: self.status.clone(),
            stage: self.stage(now_ms),
            offer_skipped: offer
                .as_ref()
                .is_some_and(|offer| self.skipped.as_ref() == Some(&offer.digest)),
            offer,
            compat_details,
            refreshing: self.manual && self.follow_ups > 0,
            problem: self.problem.clone(),
        }
    }

    fn stage(&self, now_ms: u64) -> CoreUpgradeStage {
        if self.requesting {
            return CoreUpgradeStage::Requesting;
        }
        let Some(status) = &self.status else {
            return CoreUpgradeStage::Idle;
        };
        let Some(run) = &status.run else {
            return CoreUpgradeStage::Idle;
        };
        match &run.state {
            CoreUpgradeRunState::InProgress { phase, .. } if !self.unreachable => {
                CoreUpgradeStage::InProgress {
                    phase: *phase,
                    can_cancel: *phase == CoreUpgradePhase::Pulling
                        && self.cancel_sent.as_ref() != Some(&run.id),
                }
            }
            CoreUpgradeRunState::InProgress { .. } if now_ms < self.deadline_ms => {
                CoreUpgradeStage::Reconnecting
            }
            CoreUpgradeRunState::InProgress { .. } => CoreUpgradeStage::Uncertain {
                problem: timed_out(run),
            },
            CoreUpgradeRunState::Completed => completed(status, run),
            CoreUpgradeRunState::RolledBack { problem } => CoreUpgradeStage::RolledBack {
                to: run.from.clone(),
                problem: problem.clone(),
            },
            CoreUpgradeRunState::Failed { problem } => CoreUpgradeStage::Failed {
                problem: problem.clone(),
            },
            CoreUpgradeRunState::Canceled => CoreUpgradeStage::Canceled,
            CoreUpgradeRunState::RollbackFailed { problem }
            | CoreUpgradeRunState::Unresolved { problem }
            | CoreUpgradeRunState::Unknown { problem } => CoreUpgradeStage::Uncertain {
                problem: problem.clone(),
            },
            CoreUpgradeRunState::Other => CoreUpgradeStage::Uncertain {
                problem: CoreUpgradeProblem {
                    reason: CoreUpgradeFailure::Protocol,
                    message: l10n!(CATALOG, language(), "plugins-core-upgrade-unrecognized"),
                },
            },
        }
    }

    /// 取到了核心的报告。升级中的话记下时限，阶段变了才写文件：每次轮询都写没有必要，同一阶段的时限也不变。
    fn accept(&mut self, status: CoreUpgradeStatus, now_ms: u64) -> bool {
        let phase_of = |local: &Self| match local.running_run().map(|run| (&run.id, &run.state)) {
            Some((id, CoreUpgradeRunState::InProgress { phase, .. })) => Some((id.clone(), *phase)),
            _ => None,
        };
        let before = phase_of(self);
        self.status = Some(status);
        self.unreachable = false;
        self.problem = None;
        if let Some(run) = self.running_run()
            && let CoreUpgradeRunState::InProgress { remaining_secs, .. } = run.state
        {
            self.deadline_ms = now_ms.saturating_add(remaining_secs.saturating_mul(1000));
        }
        before != phase_of(self)
    }

    /// 取状态失败了。盯着升级时连接断开是预料之中的，由 `stage` 说明，不另外报问题。
    fn reject(&mut self, problem: CoreUpgradeProblem) {
        self.unreachable = true;
        self.follow_ups = 0;
        let expected = self.running_run().is_some()
            && problem.reason == CoreUpgradeFailure::TransportUnavailable;
        self.problem = (!expected).then_some(problem);
    }

    fn saved(&self, server: String) -> Saved {
        Saved {
            server,
            watching: self
                .running_run()
                .is_some()
                .then(|| self.status.clone())
                .flatten()
                .map(|status| Watching {
                    status,
                    deadline_ms: self.deadline_ms,
                }),
            skipped: self.skipped.clone(),
        }
    }
}

/// 核心说完成了，还要它正在运行的就是目标。
fn completed(status: &CoreUpgradeStatus, run: &CoreUpgradeRun) -> CoreUpgradeStage {
    if status.current.digest.as_deref() == Some(run.to.digest.as_str()) {
        return CoreUpgradeStage::Completed { to: run.to.clone() };
    }
    let running = status.current.digest.as_deref().map_or_else(
        || l10n!(CATALOG, language(), "plugins-core-upgrade-unknown-digest"),
        |digest| digest.to_owned(),
    );
    CoreUpgradeStage::Uncertain {
        problem: CoreUpgradeProblem {
            reason: CoreUpgradeFailure::Protocol,
            message: l10n!(
                CATALOG,
                language(),
                "plugins-core-upgrade-completed-mismatch",
                running = running,
                target = run.to.digest.as_str()
            ),
        },
    }
}

fn timed_out(run: &CoreUpgradeRun) -> CoreUpgradeProblem {
    // 一条完整的话一条消息：有没有退回的线索是两句话，不拼半句。
    let message = match &run.from.digest {
        Some(digest) => l10n!(
            CATALOG,
            language(),
            "plugins-core-upgrade-timed-out-rollback",
            digest = digest
        ),
        None => l10n!(CATALOG, language(), "plugins-core-upgrade-timed-out"),
    };
    CoreUpgradeProblem {
        reason: CoreUpgradeFailure::TimedOut,
        message,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0))
}

impl PluginsLink {
    fn upgrade_file(&self) -> PathBuf {
        self.data_dir.join(SAVED_FILE)
    }

    /// App 启动：上次还盯着升级时接着盯，连上之前显示“暂时连不上”。
    pub(super) fn restore_core_upgrade(self: &Arc<Self>) {
        let server = self.home.view().server_id;
        let saved = std::fs::read(self.upgrade_file())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .filter(|saved| Some(&saved.server) == server.as_ref());
        let Some(saved) = saved else {
            return;
        };
        let watching = saved.watching.is_some();
        {
            let mut local = lock(&self.upgrade);
            local.skipped = saved.skipped;
            if let Some(watching) = saved.watching {
                local.status = Some(watching.status);
                local.deadline_ms = watching.deadline_ms;
                local.unreachable = true;
            }
        }
        self.publish_core_upgrade();
        if watching {
            self.follow_core_upgrade();
        }
    }

    pub(super) fn forget_core_upgrade(&self) {
        {
            let mut local = lock(&self.upgrade);
            let epoch = local.epoch + 1;
            *local = UpgradeLocal {
                epoch,
                polling: local.polling,
                ..UpgradeLocal::default()
            };
        }
        if let Err(error) = std::fs::remove_file(self.upgrade_file())
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("could not remove the previous server's update record: {error}");
        }
    }

    fn publish_core_upgrade(&self) {
        let view = lock(&self.upgrade).view(now_ms());
        self.view.send_if_modified(|plugins| {
            let changed = plugins.core_upgrade != view;
            plugins.core_upgrade = view;
            Self::derive(plugins) | changed
        });
    }

    /// 写不进去只影响 App 被杀后接不接得上，这次的显示不受影响。
    fn save_core_upgrade(&self) {
        let Some(server) = self.home.view().server_id else {
            return;
        };
        let local = lock(&self.upgrade);
        let path = self.upgrade_file();
        let partial = path.with_extension("json.partial");
        let written = serde_json::to_vec(&local.saved(server))
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                std::fs::create_dir_all(&self.data_dir)?;
                std::fs::write(&partial, bytes)?;
                std::fs::rename(&partial, &path)
            });
        if let Err(error) = written {
            tracing::warn!("could not write the update record: {error}");
        }
    }

    async fn upgrade_call(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<Vec<u8>>,
    ) -> Result<CoreUpgradeStatus, CoreUpgradeProblem> {
        let api = self.api().map_err(|problem| CoreUpgradeProblem {
            reason: CoreUpgradeFailure::TransportUnavailable,
            message: problem.message,
        })?;
        let timeout = if method == Method::GET {
            READ_TIMEOUT
        } else {
            SOURCE_TIMEOUT
        };
        api.core_upgrade(method, segments, body, timeout).await
    }

    /// 把取状态的结果写进去。其间发起、取消或者换了服务器时不算数。
    fn settle_core_upgrade(
        self: &Arc<Self>,
        epoch: u64,
        result: Result<CoreUpgradeStatus, CoreUpgradeProblem>,
    ) {
        let phase_changed = {
            let mut local = lock(&self.upgrade);
            if local.epoch != epoch {
                return;
            }
            match result {
                Ok(status) => {
                    let version = status.current.version.clone();
                    let changed = local.accept(status, now_ms());
                    drop(local);
                    self.server_version.seed(version);
                    changed
                }
                Err(problem) => {
                    local.reject(problem);
                    false
                }
            }
        };
        if phase_changed {
            self.save_core_upgrade();
        }
        self.publish_core_upgrade();
        self.follow_core_upgrade();
    }

    /// 只有一个在跑。退出时再发布一次：刚过时限时，要从“暂时连不上”变成“不确定”。
    fn follow_core_upgrade(self: &Arc<Self>) {
        if self.upgrade_polling_paused.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut local = lock(&self.upgrade);
            if local.polling || !local.live(now_ms()) {
                return;
            }
            local.polling = true;
        }
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            loop {
                tokio::time::sleep(POLL_INTERVAL).await;
                let epoch = {
                    let mut local = lock(&link.upgrade);
                    if link.upgrade_polling_paused.load(Ordering::SeqCst) || !local.live(now_ms()) {
                        local.polling = false;
                        break;
                    }
                    if local.requesting {
                        continue;
                    }
                    local.follow_ups = local.follow_ups.saturating_sub(1);
                    local.epoch
                };
                let fetched = link.upgrade_call(Method::GET, &PATH, None).await;
                link.settle_core_upgrade(epoch, fetched);
            }
            link.publish_core_upgrade();
        });
    }

    /// 隧道连上或回到前台：只看状态，不查新版本。
    pub(crate) fn fetch_core_upgrade(self: &Arc<Self>) {
        if self.connecting() {
            return;
        }
        let epoch = lock(&self.upgrade).epoch;
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let fetched = link.upgrade_call(Method::GET, &PATH, None).await;
            link.settle_core_upgrade(epoch, fetched);
        });
    }

    pub(crate) fn dispatch_core_upgrade(
        self: &Arc<Self>,
        action: PluginAction,
    ) -> Result<(), CoreError> {
        match action {
            PluginAction::RefreshCoreUpgrade { manual } => {
                self.refresh_core_upgrade(manual);
                Ok(())
            }
            PluginAction::StartCoreUpgrade { digest } => self.start_core_upgrade(digest),
            PluginAction::CancelCoreUpgrade => self.cancel_core_upgrade(),
            PluginAction::SkipCoreUpgrade { digest } => {
                self.skip_core_upgrade(digest);
                Ok(())
            }
            _ => Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "plugins-core-upgrade-invalid-action"),
            }),
        }
    }

    /// `RefreshCoreUpgrade`：打开升级页（`manual` 为假）或者点“检查”。
    fn refresh_core_upgrade(self: &Arc<Self>, manual: bool) {
        if self.connecting() {
            return;
        }
        let epoch = {
            let mut local = lock(&self.upgrade);
            local.manual = manual;
            local.follow_ups = CHECK_FOLLOW_UPS;
            local.epoch
        };
        self.publish_core_upgrade();
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let body = serde_json::to_vec(&CoreUpgradeCheckRequest { manual }).ok();
            let checked = link.upgrade_call(Method::POST, &CHECK_PATH, body).await;
            link.settle_core_upgrade(epoch, checked);
        });
    }

    /// `StartCoreUpgrade`。
    fn start_core_upgrade(self: &Arc<Self>, digest: String) -> Result<(), CoreError> {
        let epoch = {
            let mut local = lock(&self.upgrade);
            if local.requesting || local.running_run().is_some() {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "plugins-core-upgrade-already-running"),
                });
            }
            local.requesting = true;
            local.problem = None;
            local.epoch += 1;
            local.epoch
        };
        self.publish_core_upgrade();
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let body = serde_json::to_vec(&CoreUpgradeRequest { digest }).ok();
            let sent = link.upgrade_call(Method::POST, &PATH, body).await;
            link.finish_start(epoch, sent).await;
        });
        Ok(())
    }

    /// 发起的结果。被拒绝或者断开时重新取一次状态：`changed` 要新的版本，断开时可能已经开始了。
    async fn finish_start(
        self: &Arc<Self>,
        epoch: u64,
        sent: Result<CoreUpgradeStatus, CoreUpgradeProblem>,
    ) {
        let (fetched, refused) = match sent {
            Ok(status) => (Ok(status), None),
            Err(problem) => {
                let fetched = self.upgrade_call(Method::GET, &PATH, None).await;
                (fetched, Some(start_problem(problem)))
            }
        };
        let current_epoch = {
            let mut local = lock(&self.upgrade);
            if local.epoch != epoch {
                return;
            }
            local.requesting = false;
            local.epoch += 1;
            local.epoch
        };
        self.settle_core_upgrade(current_epoch, fetched);
        if let Some(problem) = refused {
            let mut local = lock(&self.upgrade);
            if local.running_run().is_none() {
                local.problem = Some(problem);
            }
            drop(local);
            self.publish_core_upgrade();
        }
    }

    /// `CancelCoreUpgrade`。
    fn cancel_core_upgrade(self: &Arc<Self>) -> Result<(), CoreError> {
        let (epoch, run) = {
            let mut local = lock(&self.upgrade);
            let cancellable = matches!(
                local.stage(now_ms()),
                CoreUpgradeStage::InProgress {
                    can_cancel: true,
                    ..
                }
            );
            let run = local.running_run().map(|run| run.id.clone());
            let (true, Some(run)) = (cancellable, run) else {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "plugins-core-upgrade-not-cancellable"),
                });
            };
            local.cancel_sent = Some(run.clone());
            local.epoch += 1;
            (local.epoch, run)
        };
        self.publish_core_upgrade();
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let sent = link.upgrade_call(Method::DELETE, &PATH, None).await;
            if sent.is_err() {
                let mut local = lock(&link.upgrade);
                if local.cancel_sent.as_ref() == Some(&run) {
                    local.cancel_sent = None;
                }
            }
            match sent {
                Ok(status) => link.settle_core_upgrade(epoch, Ok(status)),
                Err(problem) => {
                    lock(&link.upgrade).problem = Some(problem);
                    link.publish_core_upgrade();
                }
            }
        });
        Ok(())
    }

    /// `SkipCoreUpgrade`：记在这台设备上，渠道上出了别的版本时重新提示。
    fn skip_core_upgrade(&self, digest: String) {
        lock(&self.upgrade).skipped = Some(digest);
        self.save_core_upgrade();
        self.publish_core_upgrade();
    }
}

/// 请求断开时核心可能已经开始了，这时不知道发起了没有。
fn start_problem(problem: CoreUpgradeProblem) -> CoreUpgradeProblem {
    if problem.reason != CoreUpgradeFailure::TransportUnavailable {
        return problem;
    }
    CoreUpgradeProblem {
        reason: problem.reason,
        message: l10n!(
            CATALOG,
            language(),
            "plugins-core-upgrade-start-uncertain",
            reason = problem.message
        ),
    }
}

/// 会新失效的插件各原因一行：客户端核心按 App 的语言写（ADR-0085），壳只渲染。
/// 没查、查了没有、查不成时不产生行。
fn compat_details(offer: &CoreUpgradeOffer) -> Vec<CoreUpgradeCompatDetail> {
    let CoreUpgradeOffer {
        compat: CoreUpgradeCompat::Breaking { plugins },
        ..
    } = offer
    else {
        return Vec::new();
    };
    plugins
        .iter()
        .map(|plugin| CoreUpgradeCompatDetail {
            id: plugin.id.clone(),
            lines: plugin
                .reasons
                .iter()
                .map(|reason| match reason.category {
                    CoreUpgradeCompatCategory::SpecUnsupported => l10n!(
                        CATALOG,
                        language(),
                        "plugins-core-upgrade-compat-spec",
                        raw = reason.raw.as_str()
                    ),
                    CoreUpgradeCompatCategory::UnknownFeature => l10n!(
                        CATALOG,
                        language(),
                        "plugins-core-upgrade-compat-feature",
                        raw = reason.raw.as_str()
                    ),
                    CoreUpgradeCompatCategory::CapabilityMissing => l10n!(
                        CATALOG,
                        language(),
                        "plugins-core-upgrade-compat-capability",
                        raw = reason.raw.as_str()
                    ),
                    CoreUpgradeCompatCategory::Other if reason.raw.is_empty() => {
                        l10n!(CATALOG, language(), "plugins-core-upgrade-compat-other")
                    }
                    CoreUpgradeCompatCategory::Other => l10n!(
                        CATALOG,
                        language(),
                        "plugins-core-upgrade-compat-other-raw",
                        raw = reason.raw.as_str()
                    ),
                })
                .collect(),
        })
        .collect()
}
