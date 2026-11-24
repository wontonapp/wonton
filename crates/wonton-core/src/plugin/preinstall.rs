//! 预装：核心启动后在后台装上随核心附带的插件包，系统插件（见 [`super::system`]）和默认插件（如默认 Agent）。
//! 规则见插件规范的“预装插件”。
//!
//! 装不上、要重试时，插件表里留着一项异常写明原因，主人看得到；重试前先收回它，因为同一个 ID 只能有一项。主人在
//! 等待期间卸掉了它，就是不要它了，不再重试。卸载开始了就算，不等它卸完：到点时卸载可能还在半路，卸载失败也还是
//! 主人不要它。
//!
//! 这些插件每台主机都有，局域网状态页逐个列出它们在装还是出了什么错，不泄露主人自己装了什么（其他插件只给个数）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use wonton_protocol::EgressFailure;

use super::languages::Languages;
use super::manager::{InstallError, PluginManager, PluginStatus, PluginView, now_secs};
use super::manifest::{PluginId, PluginKind};
use super::package::PluginPackage;
use super::store::{StoreError, create_private_dir, write_json};

/// 预装失败后多久重试：从 `first` 开始每次翻倍，最多 `max`。
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub first: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    /// 刚开机时 Docker、Matrix 可能还没就绪，先很快重试；之后多半是拉镜像的网络问题，最多十分钟一次。
    fn default() -> Self {
        Self {
            first: Duration::from_secs(5),
            max: Duration::from_secs(600),
        }
    }
}

/// 出口还没就绪时最多等这么久再试：它自己会往前走，等太久反而错过就绪的那一刻（#681）。
const WAITING_MAX: Duration = Duration::from_secs(60);

/// 预装一个插件包的结果。
pub(super) enum Outcome {
    Installed,
    /// 主人在等待重试时卸掉了（或者正在卸）没装成的插件。
    Removed,
    /// 重试也没用，或者插件已经由别人在装、在处理：留给主人。
    Stopped,
}

/// 默认插件装过一次的标记：`preinstalled/<插件 id>.json`，记着当时的版本和时间，供排查。
pub struct PreinstallMarks {
    dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Mark {
    version: String,
    at: u64,
}

impl PreinstallMarks {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn contains(&self, id: &PluginId) -> Result<bool, StoreError> {
        let path = self.path(id);
        path.try_exists()
            .map_err(|source| StoreError::Io { path, source })
    }

    pub fn insert(&self, id: &PluginId, version: &str) -> Result<(), StoreError> {
        create_private_dir(&self.dir)?;
        let mark = Mark {
            version: version.to_owned(),
            at: now_secs(),
        };
        write_json(&self.path(id), &mark)
    }
}

/// `dir` 里的插件包，每个子目录一个，按目录名排序。解析不了的记一条错误后跳过；目录本身读不了时报错。
pub(super) fn packages_in(dir: &Path) -> Result<Vec<PluginPackage>, InstallError> {
    let storage = |e: std::io::Error| {
        InstallError::Storage(format!("读取插件包目录 {} 失败：{e}", dir.display()))
    };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(storage)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    let load = |path: &PathBuf| match PluginPackage::load_dir(path, &Languages::default()) {
        Ok(package) => Some(package),
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "插件包无法解析，没有安装");
            None
        }
    };
    Ok(dirs.iter().filter_map(load).collect())
}

/// 默认插件目录里的 Agent 插件，就是默认 Bot 插件（见 [`PluginManager::default_bot_plugin`]）。目录里没有
/// Agent 时为空；有不止一个时报错：哪个算默认定不下来，不按目录顺序猜。
pub fn default_bot_plugin_in(dir: &Path) -> Result<Option<PluginId>, InstallError> {
    let agents: Vec<PluginId> = packages_in(dir)?
        .into_iter()
        .filter(|package| package.manifest.kind == PluginKind::Agent)
        .map(|package| package.manifest.id)
        .collect();
    match agents.as_slice() {
        [] => Ok(None),
        [agent] => Ok(Some(agent.clone())),
        _ => Err(InstallError::Invalid(format!(
            "默认插件目录 {} 里有多个 Agent 插件（{}），定不了默认 Bot 插件",
            dir.display(),
            agents
                .iter()
                .map(PluginId::as_str)
                .collect::<Vec<_>>()
                .join("、")
        ))),
    }
}

/// 换个时间再试可能就好了的错误。插件包本身的问题重试也一样；已存在说明别人在装或在处理它。
fn retryable(e: &InstallError) -> bool {
    match e {
        InstallError::Invalid(_)
        | InstallError::PermissionNotGranted(_)
        | InstallError::Unsupported(_)
        | InstallError::UnsupportedRuntime(_)
        | InstallError::Conflict(_)
        | InstallError::SettingsInvalid(..)
        | InstallError::SettingsSchemaInvalid(..)
        | InstallError::InUse { .. }
        | InstallError::NotInstalled(_)
        | InstallError::RecordUnusable(_)
        | InstallError::NotFound(_)
        | InstallError::Changed(_)
        // 本机没有可用的显卡：装驱动、插显卡都要主人动手，下次启动再试。
        | InstallError::GpuUnavailable(_)
        // 设备不在：插上就好，下次启动再试。
        | InstallError::DeviceUnavailable(_)
        // 同名的网络或卷不是本插件的：也要主人动手。
        | InstallError::NameTaken(_)
        // 端口被占着，要主人先停掉占着它的那个。
        | InstallError::PortTaken(_) => false,
        InstallError::DockerUnavailable(_)
        | InstallError::Docker(_)
        | InstallError::DockerTimedOut(_)
        | InstallError::ImagePull(_)
        | InstallError::OidcUnavailable(_)
        | InstallError::MatrixUnavailable(_)
        | InstallError::ModelsUnavailable(_)
        | InstallError::UpstreamUnavailable(_)
        // 宿主刚启动时 DHCP 可能还没拿到地址。
        | InstallError::LanUnavailable(_)
        // 代理可能还没起来。
        | InstallError::EgressProxyUnavailable { .. }
        // 核心起不了处理图片的子进程，和插件包里的图片无关。
        | InstallError::ImagesUnavailable(_)
        | InstallError::Unhealthy(_)
        // 服务还在起：宿主空下来再试就能过（#682）。
        | InstallError::TimedOut(_)
        | InstallError::Storage(_)
        | InstallError::SourceUnavailable(_)
        | InstallError::CoreUpgrading(_) => true,
    }
}

impl PluginManager {
    /// `dir` 里的插件包（见 [`packages_in`]），记为随核心附带的插件。
    pub(super) fn bundled_in(&self, dir: &Path) -> Result<Vec<PluginPackage>, InstallError> {
        let packages = packages_in(dir)?;
        let ids = packages.iter().map(|p| p.manifest.id.clone());
        self.bundled.write().expect("插件表锁中毒").extend(ids);
        Ok(packages)
    }

    /// 默认 Bot 插件：默认安装里的 Agent 插件（[`default_bot_plugin_in`]）。家人的私人 Bot 装在它上面，“问 Bot”和
    /// 聊天列表置顶找它的默认 Bot。主人卸掉它以后仍是它：私人 Bot 如实报未安装，不改用别的已装 Agent。
    pub fn default_bot_plugin(&self) -> Option<PluginId> {
        self.default_bot_plugin
            .read()
            .expect("插件表锁中毒")
            .clone()
    }

    pub fn set_default_bot_plugin(&self, id: Option<PluginId>) {
        *self.default_bot_plugin.write().expect("插件表锁中毒") = id;
    }

    /// 随核心附带、没在运行的插件：在装、出了错或在卸载。
    pub fn bundled_not_running(&self) -> Vec<PluginView> {
        let bundled = self.bundled.read().expect("插件表锁中毒").clone();
        self.list()
            .into_iter()
            .filter(|p| bundled.contains(&p.id) && p.status != PluginStatus::Running)
            .collect()
    }

    /// 测试用：把插件记为随核心附带的。
    #[cfg(test)]
    pub(crate) fn bundled_for_test(&self, id: &PluginId) {
        self.bundled
            .write()
            .expect("插件表锁中毒")
            .insert(id.clone());
    }

    /// 预装随核心附带的插件：系统插件装完（或停下）先给模型网关的使用方对一次账（凭据的值在核心，网关的数据丢了也能
    /// 补回来）；默认插件同时装，不等它们，两边都结束后再对一次账。默认 Agent 用自己的模型接入（ADR-0108），系统插件
    /// 一直重试装不上也不能把它挡在外面（#643、#681）。
    pub async fn preinstall_bundled(
        &self,
        system: Option<&Path>,
        defaults: Option<&Path>,
        backoff: Backoff,
    ) {
        let system = async {
            if let Some(dir) = system
                && let Err(e) = self.ensure_system(dir, backoff).await
            {
                tracing::error!(error = %e, "预装系统插件失败");
            }
            for (plugin, error) in self.reconcile_models().await {
                tracing::error!(%plugin, %error, "模型网关凭据对账失败");
            }
        };
        let defaults = async {
            if let Some(dir) = defaults
                && let Err(e) = self.ensure_defaults(dir, backoff).await
            {
                tracing::error!(error = %e, "预装默认插件失败");
            }
        };
        // 两条 await 链都搬上堆：预装、升级的链贴着测试线程的栈上限（见 system.rs）。
        tokio::join!(Box::pin(system), Box::pin(defaults));
        // 默认插件装得比系统插件晚，装完再对一次账：声明了 models 的默认 Bot 这时才拿得到凭据并按新变量重建。
        for (plugin, error) in self.reconcile_models().await {
            tracing::error!(%plugin, %error, "默认插件接上模型网关失败");
        }
    }

    /// 装上 `dir` 里还没装过的默认插件，一个装完（或停下）再装下一个。系统插件不放这里。
    pub async fn ensure_defaults(&self, dir: &Path, backoff: Backoff) -> Result<(), InstallError> {
        for package in self.bundled_in(dir)? {
            let manifest = &package.manifest;
            let (id, version) = (manifest.id.clone(), manifest.version.clone());
            if manifest.kind == PluginKind::System {
                tracing::error!(plugin = %id, "默认插件目录里有系统插件，没有安装");
                continue;
            }
            match self.store.preinstalled().contains(&id) {
                Ok(true) => {
                    tracing::info!(plugin = %id, "默认插件装过一次，不再自动装");
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    // 不知道装没装过：宁可不装，免得把主人卸掉的插件装回来。
                    tracing::error!(plugin = %id, error = %e, "读取默认插件的标记失败，没有安装");
                    continue;
                }
            }
            if !self.is_installed(&id) {
                match self.preinstall(package, backoff).await {
                    Outcome::Installed | Outcome::Removed => {}
                    Outcome::Stopped => continue,
                }
            }
            if let Err(e) = self.store.preinstalled().insert(&id, &version) {
                tracing::error!(plugin = %id, error = %e, "写默认插件的标记失败");
            }
        }
        Ok(())
    }

    /// 装一个插件包，可重试的失败按 `backoff` 一直重试，直到装好、遇到重试也没用的错误或主人卸掉它。
    pub(super) async fn preinstall(&self, package: PluginPackage, backoff: Backoff) -> Outcome {
        let manifest = package.manifest.clone();
        let id = &manifest.id;
        let mut wait = backoff.first;
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let e = match self
                .install_checked(
                    package.clone(),
                    &manifest.permissions,
                    &[],
                    false,
                    &[],
                    Default::default(),
                    &self.owner_languages(),
                )
                .await
            {
                Ok(_) => {
                    tracing::info!(plugin = %id, attempts, "已预装插件");
                    return Outcome::Installed;
                }
                Err(e) => e,
            };
            // 出口还在就绪的路上：报“等着”而不是“重试”，并保持一分钟内一次的短间隔。
            let waiting = matches!(
                e,
                InstallError::EgressProxyUnavailable {
                    reason: EgressFailure::Waiting,
                    ..
                }
            );
            if let InstallError::Conflict(_) = e {
                tracing::warn!(plugin = %id, error = %e, "插件已经存在，不再预装");
                return Outcome::Stopped;
            }
            if !retryable(&e) {
                tracing::error!(plugin = %id, error = %e, "预装失败，重试也没用");
                self.mark_failed(
                    &manifest,
                    format!(
                        "{}：{e}",
                        crate::l10n::text(
                            &self.owner_languages(),
                            "plugin-install-preinstall-failed",
                            &[],
                        )
                    ),
                );
                return Outcome::Stopped;
            }
            tracing::warn!(plugin = %id, attempts, retry_in = ?wait, error = %e, "预装失败，稍后重试");
            let langs = self.owner_languages();
            let reason = format!(
                "{}：{e}",
                if waiting {
                    crate::l10n::text(&langs, "plugin-install-preinstall-waiting", &[])
                } else {
                    crate::l10n::text(
                        &langs,
                        "plugin-install-preinstall-retrying",
                        &[("attempts", attempts.to_string().into())],
                    )
                }
            );
            // 先订阅再标异常：标完之后主人马上卸掉它，也能收到通知。
            let changes = self.changes();
            self.mark_failed(&manifest, reason.clone());
            if !self.wait_to_retry(id, &reason, wait, changes).await {
                return match self.status_of(id) {
                    None | Some(PluginStatus::Uninstalling) => {
                        tracing::info!(plugin = %id, "主人卸掉了没装成的插件，不再预装");
                        Outcome::Removed
                    }
                    Some(_) => {
                        tracing::info!(plugin = %id, "插件的状态变了，不再预装");
                        Outcome::Stopped
                    }
                };
            }
            wait = if waiting {
                (wait * 2).min(WAITING_MAX).max(backoff.first)
            } else {
                (wait * 2).min(backoff.max)
            };
        }
    }

    /// 等 `wait` 之后收回自己标的异常，准备重试。插件表在这期间变了、这个插件不再是 `reason` 这个异常（主人卸掉了它，
    /// 或者正在处理它），就提前返回 false。
    async fn wait_to_retry(
        &self,
        id: &PluginId,
        reason: &str,
        wait: Duration,
        mut changes: tokio::sync::watch::Receiver<u64>,
    ) -> bool {
        let deadline = tokio::time::sleep(wait);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                () = &mut deadline => break,
                changed = changes.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    if !self.is_failed_with(id, reason) {
                        return false;
                    }
                }
            }
        }
        self.take_failed(id, reason)
    }
}
