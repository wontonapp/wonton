//! 核心这边的升级：查新版本、发起、取消，看住升级器，升级没结束时冻结插件（ADR-0025）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::watch;
use wonton_protocol::{
    CoreUpgradeCheck, CoreUpgradeFailure as Failure, CoreUpgradeStatus, CoreVersion,
};

use super::check::{channel, version_of};
use super::host::{Host, HostError, Mount, UpdaterSpec};
use super::state::{self, ImageRef, Layout, Outcome, UpgradeState, WATCHDOG_MARGIN, problem};
use super::{UPDATER_CONTAINER, backup};
use crate::l10n;
use crate::plugin::languages::Languages;

/// 多久看一次升级器和该不该查新版本。
const TICK: Duration = Duration::from_secs(10);
/// 定时检查的间隔（已定 2）。
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// 正式安装时 compose 传进来的设置。
#[derive(Debug, Clone)]
pub struct UpgradeConfig {
    /// `WONTON_CORE_SOURCE`。
    pub source: Option<String>,
    /// `WONTON_INSTALL_DIR`。
    pub install_dir: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub core_container: Option<String>,
    /// 设了实例名的核心不能一键升级（ADR-0011）。
    pub has_instance: bool,
}

impl UpgradeConfig {
    fn is_installed(&self) -> bool {
        self.source.is_some()
            && self.install_dir.is_some()
            && self.core_container.is_some()
            && !self.has_instance
    }
}

/// 接口拒绝的原因。
#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    #[error("{0}")]
    Changed(String),
    #[error("{0}")]
    NotReady(String),
    #[error("{0}")]
    Busy(String),
    #[error("{0}")]
    TooLate(String),
    #[error("{0}")]
    Upstream(String),
    #[error("{0:#}")]
    Storage(anyhow::Error),
}

impl UpgradeError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Changed(_) => "changed",
            Self::NotReady(_) => "not_ready",
            Self::Busy(_) => "busy",
            Self::TooLate(_) => "too_late",
            Self::Upstream(_) => "upstream",
            Self::Storage(_) => "storage",
        }
    }
}

impl From<HostError> for UpgradeError {
    fn from(e: HostError) -> Self {
        Self::Upstream(e.to_string())
    }
}

/// 正在运行的核心。
#[derive(Debug, Clone)]
pub struct Running {
    pub image_id: String,
    pub config_image: String,
    pub digest: Option<String>,
    pub version: Option<String>,
    pub size: u64,
}

pub struct Upgrades {
    pub(super) config: UpgradeConfig,
    pub(super) host: Arc<dyn Host>,
    check: Mutex<CoreUpgradeCheck>,
    checking: AtomicBool,
    next_check: Mutex<u64>,
    starting: tokio::sync::Mutex<()>,
    running: OnceLock<Running>,
    frozen: watch::Sender<bool>,
}

impl Upgrades {
    pub fn new(config: UpgradeConfig, host: Arc<dyn Host>) -> Self {
        let check = match config.source.as_deref() {
            _ if !config.is_installed() => CoreUpgradeCheck::NotInstalled,
            Some(source) if channel(source).is_none() => CoreUpgradeCheck::Pinned,
            _ => CoreUpgradeCheck::NotChecked,
        };
        let this = Self {
            config,
            host,
            check: Mutex::new(check),
            checking: AtomicBool::new(false),
            next_check: Mutex::new(0),
            starting: tokio::sync::Mutex::new(()),
            running: OnceLock::new(),
            frozen: watch::Sender::new(false),
        };
        // 插件管理器恢复之前就要知道：新核心可能正处在升级的观察期里。
        match state::load(&this.layout().state_path(), &this.owner_languages()) {
            Ok(Some(last)) => this.frozen.send_replace(last.freezes_plugins()),
            Ok(None) => false,
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "读不懂升级状态文件，当作没有在升级");
                false
            }
        };
        this
    }

    /// 后台自己写文字（检查、看住升级器）用主人最近一次请求的语言（ADR-0043）。
    pub(super) fn owner_languages(&self) -> Languages {
        crate::plugin::store::read_owner_languages(&self.config.data_dir)
    }

    pub(super) fn layout(&self) -> Layout {
        Layout {
            install_dir: self.config.install_dir.clone().unwrap_or_default(),
            data_dir: self.config.data_dir.clone(),
        }
    }

    /// 升级没结束时是真：插件管理器据此拒绝安装和卸载。
    pub fn frozen(&self) -> watch::Receiver<bool> {
        self.frozen.subscribe()
    }

    /// 等到不再冻结。预装插件要等它：升级没结束时不预装（ADR-0025）。
    pub async fn wait_released(&self) {
        let mut frozen = self.frozen.subscribe();
        // 发送端活得和 self 一样久，这里的 wait_for 不会失败。
        let _ = frozen.wait_for(|frozen| !*frozen).await;
    }

    pub(super) fn set_check(&self, check: CoreUpgradeCheck) {
        *self.check.lock().expect("升级检查的锁中毒") = check;
    }

    fn current_check(&self) -> CoreUpgradeCheck {
        self.check.lock().expect("升级检查的锁中毒").clone()
    }

    pub(super) async fn running(&self, langs: &Languages) -> Result<Running, UpgradeError> {
        if let Some(running) = self.running.get() {
            return Ok(running.clone());
        }
        let Some(name) = self.config.core_container.as_deref() else {
            return Err(UpgradeError::NotReady(l10n::text(
                langs,
                "upgrade-not-in-container",
                &[],
            )));
        };
        let container = self.host.container(name).await?.ok_or_else(|| {
            UpgradeError::Upstream(l10n::text(
                langs,
                "upgrade-container-missing",
                &[("name", name.into())],
            ))
        })?;
        let info = self.host.image(&container.image_id).await?.ok_or_else(|| {
            UpgradeError::Upstream(l10n::text(
                langs,
                "upgrade-image-missing",
                &[("image", container.image_id.as_str().into())],
            ))
        })?;
        let digest = match container.config_image.split_once('@') {
            Some((_, digest)) => Some(digest.to_owned()),
            None => info
                .repo_digests
                .first()
                .and_then(|r| r.split_once('@').map(|(_, digest)| digest.to_owned())),
        };
        let running = Running {
            image_id: container.image_id,
            config_image: container.config_image,
            digest,
            version: version_of(&info),
            size: info.size,
        };
        Ok(self.running.get_or_init(|| running).clone())
    }

    /// 这台核心的版本号，就是升级接口报的那个。不在容器里、镜像没有版本号时为空。
    pub async fn running_version(&self) -> Option<String> {
        self.running(&self.owner_languages()).await.ok()?.version
    }

    fn read_state(&self, langs: &Languages) -> Result<Option<UpgradeState>, UpgradeError> {
        state::load(&self.layout().state_path(), langs).map_err(UpgradeError::Storage)
    }

    pub async fn status(&self, langs: &Languages) -> Result<CoreUpgradeStatus, UpgradeError> {
        let current = match self.running(langs).await {
            Ok(running) => CoreVersion {
                version: running.version,
                digest: running.digest,
            },
            Err(UpgradeError::NotReady(_)) => CoreVersion {
                version: None,
                digest: None,
            },
            Err(e) => return Err(e),
        };
        let channel = self.config.source.as_deref().and_then(channel);
        let now = self.host.now();
        Ok(CoreUpgradeStatus {
            current,
            channel: channel.map(|(_, tag)| tag.to_owned()),
            check: self.current_check(),
            run: self.read_state(langs)?.map(|last| last.run(now, langs)),
        })
    }

    /// 在后台查一次新版本。已经在查、不跟渠道、有没结束的升级时什么都不做。
    pub fn trigger_check(self: &Arc<Self>, manual: bool) {
        if !self.may_check() || self.checking.swap(true, Ordering::SeqCst) {
            return;
        }
        // 复查期间保留上次的结果：确认页上的新版本不能闪一下没了。
        if matches!(self.current_check(), CoreUpgradeCheck::NotChecked) {
            self.set_check(CoreUpgradeCheck::Checking);
        }
        let this = self.clone();
        tokio::spawn(async move {
            this.check_now(manual).await;
            this.checking.store(false, Ordering::SeqCst);
        });
    }

    fn may_check(&self) -> bool {
        let follows = self.config.source.as_deref().and_then(channel).is_some();
        let idle = self
            .read_state(&self.owner_languages())
            .is_ok_and(|last| last.is_none_or(|s| s.is_ended()));
        self.config.is_installed() && follows && idle
    }

    /// 发起升级。`digest` 是主人确认的版本。
    pub async fn start(
        &self,
        digest: &str,
        langs: &Languages,
    ) -> Result<CoreUpgradeStatus, UpgradeError> {
        if !self.config.is_installed() {
            return Err(UpgradeError::NotReady(l10n::text(
                langs,
                "upgrade-not-installed",
                &[],
            )));
        }
        let _serial = self.starting.lock().await;
        self.supervise().await;
        let last = self.read_state(langs)?;
        if last.as_ref().is_some_and(|s| !s.is_ended()) {
            return Err(UpgradeError::Busy(l10n::text(langs, "upgrade-busy", &[])));
        }
        let offer = match self.current_check() {
            CoreUpgradeCheck::Ready { offer } if offer.digest == digest => offer,
            CoreUpgradeCheck::Ready { .. } | CoreUpgradeCheck::UpToDate => {
                return Err(UpgradeError::Changed(l10n::text(
                    langs,
                    "upgrade-channel-changed",
                    &[],
                )));
            }
            CoreUpgradeCheck::Downloading { digest: new, .. }
            | CoreUpgradeCheck::DownloadFailed { digest: new, .. }
                if new != digest =>
            {
                return Err(UpgradeError::Changed(l10n::text(
                    langs,
                    "upgrade-channel-changed",
                    &[],
                )));
            }
            _ => {
                return Err(UpgradeError::NotReady(l10n::text(
                    langs,
                    "upgrade-not-downloaded",
                    &[],
                )));
            }
        };
        let running = self.running(langs).await?;
        let (repository, _) = self
            .config
            .source
            .as_deref()
            .and_then(channel)
            .ok_or_else(|| UpgradeError::NotReady(l10n::text(langs, "upgrade-pinned", &[])))?;
        let mut fresh = UpgradeState::new(
            crate::secret::random_token(),
            ImageRef {
                image: running.config_image.clone(),
                digest: running.digest.clone(),
                version: running.version.clone(),
                image_id: Some(running.image_id.clone()),
            },
            ImageRef {
                image: format!("{repository}@{}", offer.digest),
                digest: Some(offer.digest.clone()),
                version: offer.version.clone(),
                image_id: None,
            },
            offer.migrates.clone(),
            self.host.now(),
        );
        fresh.stale_images = last.map(|s| s.rollback_images).unwrap_or_default();
        self.launch(fresh, &running, langs).await?;
        self.status(langs).await
    }

    async fn launch(
        &self,
        mut fresh: UpgradeState,
        running: &Running,
        langs: &Languages,
    ) -> Result<(), UpgradeError> {
        let layout = self.layout();
        self.host.remove_container(UPDATER_CONTAINER).await?;
        self.remove_cancel_markers();
        state::save(&layout.state_path(), &fresh, langs).map_err(UpgradeError::Storage)?;
        let spec = UpdaterSpec {
            image_id: running.image_id.clone(),
            cmd: vec![
                "updater".to_owned(),
                layout.install_dir.display().to_string(),
                layout.data_dir.display().to_string(),
            ],
            mounts: vec![
                Mount::same_path(std::path::Path::new("/var/run/docker.sock"), false),
                Mount::same_path(&layout.install_dir, false),
                Mount::same_path(&layout.core_dir(), false),
            ],
        };
        if let Err(e) = self.host.start_updater(&spec).await {
            // 什么都没动：如实写成失败，这条路也有结果。
            fresh.result = Some(Outcome::Failed);
            let message = l10n::text(
                langs,
                "upgrade-launch-updater-failed",
                &[("error", e.to_string().into())],
            );
            fresh.problem = Some(problem(Failure::UpdaterFailed, message));
            state::save(&layout.state_path(), &fresh, langs).map_err(UpgradeError::Storage)?;
        }
        Ok(())
    }

    fn remove_cancel_markers(&self) {
        let Ok(entries) = std::fs::read_dir(self.layout().upgrade_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("cancel-")
                && let Err(e) = std::fs::remove_file(entry.path())
            {
                tracing::warn!(error = %e, "删除旧的取消标记失败");
            }
        }
    }

    /// 请求取消。结果以状态文件为准：`canceled`，或者已经往下走了。
    pub async fn cancel(&self, langs: &Languages) -> Result<CoreUpgradeStatus, UpgradeError> {
        let Some(last) = self.read_state(langs)?.filter(|s| !s.is_ended()) else {
            return Err(UpgradeError::TooLate(l10n::text(
                langs,
                "upgrade-not-upgrading",
                &[],
            )));
        };
        if last.phase != wonton_protocol::CoreUpgradePhase::Pulling {
            return Err(UpgradeError::TooLate(l10n::text(
                langs,
                "upgrade-cancel-too-late",
                &[],
            )));
        }
        let path = self.layout().cancel_path(&last.id);
        state::write_atomic(&path, b"", 0o600).map_err(UpgradeError::Storage)?;
        self.status(langs).await
    }

    /// 看一眼这次升级：更新冻结的状态；升级器不在了就写结果，每条路最后都要有结果。
    pub async fn supervise(&self) {
        let langs = self.owner_languages();
        let last = match self.read_state(&langs) {
            Ok(Some(last)) => last,
            Ok(None) => {
                self.frozen.send_replace(false);
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "读不懂升级状态文件");
                return;
            }
        };
        self.frozen.send_replace(last.freezes_plugins());
        if last.is_ended() || !self.config.is_installed() {
            return;
        }
        match self.host.container(UPDATER_CONTAINER).await {
            Ok(None) => self.updater_gone(&last.id),
            Ok(Some(updater)) if !updater.running && updater.restart_disabled => {
                self.updater_gone(&last.id);
            }
            Ok(Some(updater)) => {
                self.watchdog(&last, updater.running, updater.restart_disabled)
                    .await
            }
            // 查不了容器时不下结论：宿主刚重启时它可能还没被 Docker 拉起来。
            Err(e) => tracing::warn!(error = %e, "查升级器容器失败"),
        }
    }

    /// 升级器跑不到自己的判断时（比如启动就崩）的兜底。还在运行就先不动它：compose 做到一半被杀更糟。
    async fn watchdog(&self, last: &UpgradeState, running: bool, restart_disabled: bool) {
        let now = self.host.now();
        let margin = WATCHDOG_MARGIN.as_secs();
        let limit = last.phase_limit_at();
        if now <= limit + margin {
            return;
        }
        if !restart_disabled && let Err(e) = self.host.disable_restart(UPDATER_CONTAINER).await {
            tracing::error!(error = %e, "把升级器的重启策略改成 no 失败");
        }
        if running && now > limit + 2 * margin {
            tracing::error!(phase = ?last.phase, "升级器卡死了，停掉它");
            match self.host.stop(UPDATER_CONTAINER).await {
                Ok(()) => self.updater_gone(&last.id),
                Err(e) => tracing::error!(error = %e, "停升级器失败"),
            }
        }
    }

    /// 查完容器后重读：升级器先写结果再改重启策略、再退出，这样盖不掉它写的结果。
    fn updater_gone(&self, id: &str) {
        let langs = self.owner_languages();
        let path = self.layout().state_path();
        let write = || -> anyhow::Result<()> {
            let Some(mut last) = state::load(&path, &langs)? else {
                return Ok(());
            };
            if last.id != id || last.is_ended() {
                return Ok(());
            }
            if let Some(dir) = &last.backup_dir
                && dir.exists()
            {
                backup::mark_keep(dir)?;
            }
            last.result = Some(Outcome::Unknown);
            let message = l10n::text(
                &langs,
                "upgrade-updater-gone",
                &[("phase", format!("{:?}", last.phase).into())],
            );
            last.problem = Some(problem(Failure::UpdaterFailed, message));
            state::save(&path, &last, &langs)
        };
        match write() {
            Ok(()) => {
                self.frozen.send_replace(false);
            }
            Err(e) => tracing::error!(error = %format!("{e:#}"), "写升级结果失败"),
        }
    }

    fn check_due(&self) -> bool {
        let now = self.host.now();
        let mut next = self.next_check.lock().expect("升级检查的锁中毒");
        if now < *next || !self.may_check() {
            return false;
        }
        *next = now + CHECK_INTERVAL.as_secs();
        true
    }

    /// 后台：看住升级器，启动时和之后每 6 小时查一次新版本。
    pub fn spawn(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                this.supervise().await;
                if this.check_due() {
                    this.trigger_check(false);
                }
                this.host.sleep(TICK).await;
            }
        });
    }
}
