//! 插件管理器：维护已安装插件的状态，对外提供预览、安装、卸载和网关路由查询。
//! 具体的 Docker 部署和回滚在 [`super::lifecycle`] 里。

use std::collections::BTreeMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::docker::{Engine, EngineError};
use super::health::HealthError;
use super::lifecycle::LifecycleConfig;
use super::manifest::{Arch, LocalizedText, Manifest, Permission, PluginId, PluginKind, Sso};
use super::package::{PackageError, PluginPackage};
use super::render::{RenderError, dry_run};
use super::secrets::CollectError;
use super::store::{InstalledRecord, Store, StoreError};
use crate::http::HttpClient;
use crate::oidc::{Hydra, OidcError};

/// 插件管理操作的错误，按原因区分，API 据此给出不同的 HTTP 状态。
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{0}")]
    Invalid(String),
    #[error("以下权限需要主人确认后才能安装：{}", .0.join(", "))]
    PermissionNotGranted(Vec<String>),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Conflict(String),
    #[error("插件 {0} 没有安装")]
    NotInstalled(String),
    #[error("Docker 不可用：{0}")]
    DockerUnavailable(String),
    #[error("Docker 操作失败：{0}")]
    Docker(String),
    #[error("身份提供方不可用：{0}")]
    OidcUnavailable(String),
    #[error("插件没有通过健康检查：{0}")]
    Unhealthy(String),
    #[error("{0}")]
    Storage(String),
}

impl InstallError {
    /// 机器可读的错误类别。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_input",
            Self::PermissionNotGranted(_) => "permission_denied",
            Self::Unsupported(_) => "unsupported",
            Self::Conflict(_) => "conflict",
            Self::NotInstalled(_) => "not_installed",
            Self::DockerUnavailable(_) => "docker_unavailable",
            Self::Docker(_) => "docker_rejected",
            Self::OidcUnavailable(_) => "oidc_unavailable",
            Self::Unhealthy(_) => "unhealthy",
            Self::Storage(_) => "storage",
        }
    }
}

impl InstallError {
    /// 在原错误后追加说明，类别不变（如回滚失败的残留信息）。
    pub fn with_note(self, note: &str) -> Self {
        let add = |message: String| format!("{message}（{note}）");
        match self {
            Self::Invalid(m) => Self::Invalid(add(m)),
            Self::PermissionNotGranted(list) => Self::PermissionNotGranted(list),
            Self::Unsupported(m) => Self::Unsupported(add(m)),
            Self::Conflict(m) => Self::Conflict(add(m)),
            Self::NotInstalled(m) => Self::NotInstalled(m),
            Self::DockerUnavailable(m) => Self::DockerUnavailable(add(m)),
            Self::Docker(m) => Self::Docker(add(m)),
            Self::OidcUnavailable(m) => Self::OidcUnavailable(add(m)),
            Self::Unhealthy(m) => Self::Unhealthy(add(m)),
            Self::Storage(m) => Self::Storage(add(m)),
        }
    }
}

impl From<PackageError> for InstallError {
    fn from(e: PackageError) -> Self {
        Self::Invalid(e.to_string())
    }
}

impl From<RenderError> for InstallError {
    fn from(e: RenderError) -> Self {
        match e {
            RenderError::Unsupported(message) => Self::Unsupported(message),
            other => Self::Invalid(other.to_string()),
        }
    }
}

impl From<EngineError> for InstallError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::Unavailable(m) => Self::DockerUnavailable(m),
            EngineError::Unsupported(m) => Self::Unsupported(m),
            other => Self::Docker(other.to_string()),
        }
    }
}

impl From<HealthError> for InstallError {
    fn from(e: HealthError) -> Self {
        match e {
            HealthError::Engine(engine) => engine.into(),
            other => Self::Unhealthy(other.to_string()),
        }
    }
}

impl From<OidcError> for InstallError {
    fn from(e: OidcError) -> Self {
        Self::OidcUnavailable(e.to_string())
    }
}

impl From<CollectError> for InstallError {
    fn from(e: CollectError) -> Self {
        match e {
            CollectError::Engine(engine) => engine.into(),
            invalid @ CollectError::Invalid { .. } => Self::Invalid(invalid.to_string()),
        }
    }
}

impl From<StoreError> for InstallError {
    fn from(e: StoreError) -> Self {
        Self::Storage(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PluginStatus {
    Installing,
    Running,
    /// 已安装但状态异常（容器缺失或停止、卸载中途失败等），原因如实给出。
    Degraded {
        reason: String,
    },
    Uninstalling,
}

/// 运行中的插件健不健康。核心在安装时和每次启动时检查，这里是最近一次检查的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// 每个长期运行的服务都有健康检查，而且都通过了。
    Healthy,
    /// 有长期运行的服务没有健康检查（或者检查时还没通过）：只知道它在运行，不能说它健康。
    Unknown,
}

impl Health {
    /// 所有长期运行的服务都健康才算健康。
    pub fn of(healthy: impl IntoIterator<Item = bool>) -> Self {
        if healthy.into_iter().all(|h| h) {
            Self::Healthy
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginView {
    pub id: PluginId,
    pub version: String,
    pub title: LocalizedText,
    pub status: PluginStatus,
    /// 只有运行中的插件才有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionView {
    pub permission: Permission,
    pub high: bool,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub id: PluginId,
    pub version: String,
    pub kind: PluginKind,
    pub title: LocalizedText,
    pub description: LocalizedText,
    pub permissions: Vec<PermissionView>,
    pub services: Vec<String>,
    pub url: Option<String>,
    pub sso: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UninstallReport {
    pub containers: Vec<String>,
    pub networks: Vec<String>,
    pub volumes: Vec<String>,
    pub data_removed: bool,
    /// 没有配置身份提供方时为空。
    pub oidc_client_removed: Option<bool>,
}

/// 网关查询用的路由信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub id: PluginId,
    pub status: PluginStatus,
    /// `host:port`，核心可直接访问的插件网页地址。
    pub upstream: Option<String>,
}

struct Entry {
    version: String,
    title: LocalizedText,
    has_webui: bool,
    status: PluginStatus,
    health: Option<Health>,
    upstream: Option<String>,
}

/// 部署或核对完成的插件：核心访问它网页的地址，和检查时的健康状况。
pub(super) struct Deployed {
    pub upstream: Option<String>,
    pub health: Health,
}

impl Entry {
    fn new(manifest: &Manifest, status: PluginStatus, upstream: Option<String>) -> Self {
        Self {
            version: manifest.version.clone(),
            title: manifest.title.clone(),
            has_webui: manifest.webui.is_some(),
            status,
            health: None,
            upstream,
        }
    }

    fn running(manifest: &Manifest, deployed: Deployed) -> Self {
        Self {
            health: Some(deployed.health),
            ..Self::new(manifest, PluginStatus::Running, deployed.upstream)
        }
    }

    /// 记录或其中的插件包读不出来：只知道 ID（版本可能也不知道），如实标为异常，主人可以卸载它。
    fn broken(id: &PluginId, version: String, reason: String) -> Self {
        Self {
            version,
            title: ["zh", "en"]
                .map(|lang| (lang.to_owned(), id.to_string()))
                .into(),
            has_webui: false,
            status: PluginStatus::Degraded { reason },
            health: None,
            upstream: None,
        }
    }
}

pub struct PluginManager {
    pub(super) engine: Engine,
    pub(super) store: Store,
    pub(super) http: HttpClient,
    pub(super) oidc: Option<Hydra>,
    pub(super) cfg: LifecycleConfig,
    plugins: RwLock<BTreeMap<PluginId, Entry>>,
}

/// 核心已经兑现的权限。其余权限在规范里定义了，但核心还不能兑现，安装时如实拒绝。
fn supported(permission: &Permission) -> bool {
    matches!(
        permission,
        Permission::StorageShare { .. } | Permission::NetworkEgress
    )
}

pub(super) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl PluginManager {
    pub fn new(
        engine: Engine,
        store: Store,
        http: HttpClient,
        oidc: Option<Hydra>,
        cfg: LifecycleConfig,
    ) -> Self {
        Self {
            engine,
            store,
            http,
            oidc,
            cfg,
            plugins: RwLock::new(BTreeMap::new()),
        }
    }

    /// 预览只有 compose 文本的插件包。
    pub fn preview(&self, source: &str) -> Result<Preview, InstallError> {
        self.preview_package(&PluginPackage::parse(source)?)
    }

    pub fn preview_package(&self, package: &PluginPackage) -> Result<Preview, InstallError> {
        dry_run(package)?;
        let m = package.manifest.clone();
        let slug = m.id.slug();
        Ok(Preview {
            permissions: m
                .permissions
                .iter()
                .map(|p| PermissionView {
                    permission: p.clone(),
                    high: p.is_high(),
                    supported: supported(p),
                })
                .collect(),
            services: package.service_names().map(str::to_owned).collect(),
            url: m
                .webui
                .as_ref()
                .map(|_| self.cfg.public.origin(&self.cfg.public.plugin_host(&slug))),
            sso: m.webui.as_ref().is_some_and(|w| w.sso == Sso::Oidc),
            id: m.id,
            version: m.version,
            kind: m.kind,
            title: m.title,
            description: m.description,
        })
    }

    pub fn list(&self) -> Vec<PluginView> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .map(|(id, entry)| self.view(id, entry))
            .collect()
    }

    pub fn route(&self, slug: &str) -> Option<Route> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .find(|(id, _)| id.slug() == slug)
            .map(|(id, entry)| Route {
                id: id.clone(),
                status: entry.status.clone(),
                upstream: entry.upstream.clone(),
            })
    }

    fn view(&self, id: &PluginId, entry: &Entry) -> PluginView {
        PluginView {
            id: id.clone(),
            version: entry.version.clone(),
            title: entry.title.clone(),
            status: entry.status.clone(),
            health: entry.health,
            url: entry.has_webui.then(|| {
                self.cfg
                    .public
                    .origin(&self.cfg.public.plugin_host(&id.slug()))
            }),
        }
    }

    /// 测试用：不经 Docker 直接登记一个插件，供网关测试指向本地的假上游。
    #[cfg(test)]
    pub(crate) fn register_for_test(
        &self,
        manifest: Manifest,
        status: PluginStatus,
        upstream: Option<String>,
    ) {
        let entry = Entry::new(&manifest, status, upstream);
        self.plugins
            .write()
            .expect("插件表锁中毒")
            .insert(manifest.id, entry);
    }

    fn set(&self, id: &PluginId, status: PluginStatus, upstream: Option<Option<String>>) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id) {
            entry.status = status;
            entry.health = None;
            if let Some(upstream) = upstream {
                entry.upstream = upstream;
            }
        }
    }

    /// 核心启动时：按记录恢复插件状态，把 OIDC 客户端同步到身份提供方。
    /// 单个插件恢复失败（包括它的记录损坏）只把它标为异常，不影响核心和其他插件启动。
    pub async fn restore(&self) -> Result<(), InstallError> {
        for file in self.store.load_all()? {
            let entry = match &file.record {
                Ok(record) => self.restore_one(record).await,
                Err(e) => Entry::broken(&file.id, String::new(), e.to_string()),
            };
            match &entry.status {
                PluginStatus::Running => tracing::info!(plugin = %file.id, "已恢复插件"),
                status => tracing::warn!(plugin = %file.id, ?status, "插件状态异常"),
            }
            self.plugins
                .write()
                .expect("插件表锁中毒")
                .insert(file.id, entry);
        }
        Ok(())
    }

    async fn restore_one(&self, record: &InstalledRecord) -> Entry {
        match PluginPackage::with_files(&record.source, record.files.clone()) {
            Ok(package) => match self.reconcile(&package, record).await {
                Ok(deployed) => Entry::running(&package.manifest, deployed),
                Err(e) => {
                    let reason = e.to_string();
                    Entry::new(&package.manifest, PluginStatus::Degraded { reason }, None)
                }
            },
            Err(e) => Entry::broken(
                &record.id,
                record.version.clone(),
                format!("记录里的插件包无法解析：{e}"),
            ),
        }
    }

    /// 安装只有 compose 文本的插件包，见 [`Self::install_package`]。
    pub async fn install(
        &self,
        source: &str,
        granted: &[Permission],
    ) -> Result<PluginView, InstallError> {
        self.install_package(PluginPackage::parse(source)?, granted)
            .await
    }

    /// 安装插件。`granted` 必须与清单声明的权限完全一致：这就是主人的确认。
    pub async fn install_package(
        &self,
        package: PluginPackage,
        granted: &[Permission],
    ) -> Result<PluginView, InstallError> {
        dry_run(&package)?;
        let manifest = package.manifest.clone();
        check_permissions(&manifest, granted)?;
        self.check_requirements(&manifest).await?;
        self.reserve(&manifest)?;
        tracing::info!(plugin = %manifest.id, "开始安装插件");
        match self.deploy(&package, granted).await {
            Ok(deployed) => {
                tracing::info!(plugin = %manifest.id, health = ?deployed.health, "插件安装完成");
                let mut plugins = self.plugins.write().expect("插件表锁中毒");
                let entry = Entry::running(&manifest, deployed);
                let view = self.view(&manifest.id, &entry);
                plugins.insert(manifest.id.clone(), entry);
                Ok(view)
            }
            Err(e) => {
                tracing::warn!(plugin = %manifest.id, error = %e, "插件安装失败，已回滚");
                self.plugins
                    .write()
                    .expect("插件表锁中毒")
                    .remove(&manifest.id);
                Err(e)
            }
        }
    }

    /// 占位，防止同一插件并发安装，也防止两个插件映射到同一个子域名。
    fn reserve(&self, manifest: &Manifest) -> Result<(), InstallError> {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(existing) = plugins.get(&manifest.id) {
            return Err(InstallError::Conflict(format!(
                "插件 {} 已存在（状态：{:?}）",
                manifest.id, existing.status
            )));
        }
        let slug = manifest.id.slug();
        if let Some(other) = plugins.keys().find(|id| id.slug() == slug) {
            return Err(InstallError::Conflict(format!(
                "插件 {} 与已安装的 {other} 使用同一个网页地址",
                manifest.id
            )));
        }
        plugins.insert(
            manifest.id.clone(),
            Entry::new(manifest, PluginStatus::Installing, None),
        );
        Ok(())
    }

    async fn check_requirements(&self, manifest: &Manifest) -> Result<(), InstallError> {
        if manifest.kind == PluginKind::System {
            return Err(InstallError::Unsupported(
                "system 插件只能由安装器预装".to_owned(),
            ));
        }
        let arch = self.engine.host_arch().await?;
        let wanted = manifest.requirements.as_ref().and_then(|r| r.arch.as_ref());
        match (wanted, Arch::from_docker(&arch)) {
            (Some(list), Some(host)) if !list.contains(&host) => Err(InstallError::Unsupported(
                format!("插件不支持本机架构 {arch}"),
            )),
            (Some(_), None) => Err(InstallError::Unsupported(format!(
                "无法识别本机架构 {arch}"
            ))),
            _ => Ok(()),
        }
    }

    /// 卸载插件。`purge` 时一并删除命名卷和应用数据目录。
    pub async fn uninstall(
        &self,
        id: &PluginId,
        purge: bool,
    ) -> Result<UninstallReport, InstallError> {
        {
            let mut plugins = self.plugins.write().expect("插件表锁中毒");
            let entry = plugins
                .get_mut(id)
                .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
            if matches!(
                entry.status,
                PluginStatus::Installing | PluginStatus::Uninstalling
            ) {
                return Err(InstallError::Conflict(format!(
                    "插件 {id} 正在{}",
                    if entry.status == PluginStatus::Installing {
                        "安装"
                    } else {
                        "卸载"
                    }
                )));
            }
            entry.status = PluginStatus::Uninstalling;
        }
        tracing::info!(plugin = %id, purge, "开始卸载插件");
        match self.teardown(id, purge).await {
            Ok(report) => {
                self.plugins.write().expect("插件表锁中毒").remove(id);
                tracing::info!(plugin = %id, "插件已卸载");
                Ok(report)
            }
            Err(e) => {
                let reason = format!("卸载未完成：{e}");
                self.set(id, PluginStatus::Degraded { reason }, Some(None));
                Err(e)
            }
        }
    }
}

fn check_permissions(manifest: &Manifest, granted: &[Permission]) -> Result<(), InstallError> {
    if let Some(extra) = granted.iter().find(|p| !manifest.permissions.contains(p)) {
        return Err(InstallError::Invalid(format!(
            "授予了清单没有声明的权限 {extra}"
        )));
    }
    let missing: Vec<String> = manifest
        .permissions
        .iter()
        .filter(|p| !granted.contains(p))
        .map(ToString::to_string)
        .collect();
    if !missing.is_empty() {
        return Err(InstallError::PermissionNotGranted(missing));
    }
    if let Some(p) = manifest.permissions.iter().find(|p| !supported(p)) {
        return Err(InstallError::Unsupported(format!(
            "权限 {p} 在 M0 尚未支持"
        )));
    }
    Ok(())
}
