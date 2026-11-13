//! 模型网关（插件规范的“模型网关”）。提供方是声明了 `provides.models` 的系统插件，同一时间只有一个；声明了
//! `models` 的插件经核心在插件网络里的别名 `models.wonton.internal` 访问它，各自拿一个只属于自己的凭据。
//!
//! 凭据按 Docker 凭据助手的约定签发：核心在网关的容器里执行声明的命令，值走标准输入，只看退出码；签发后用这个
//! 凭据请求一次 `GET <openai>/models`，返回 200 才算成功。值存在插件记录目录下的 `model-keys/`，网关的数据丢了或
//! 重装了，核心用同一个值重新签发。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use wonton_protocol::ModelSettings;

use super::health_gate::Gate;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::{Manifest, Permission, PluginId};
use super::package::PluginPackage;
use super::provides::{ModelsProvider, NAME_PLACEHOLDER};
use super::render;
use super::store::{
    InstalledRecord, StoreError, create_private_dir, read_json, remove_file, write_json,
};
use super::vars::ModelVars;
use crate::config::PLUGIN_MODELS_HOST;
use crate::secret::random_token;

/// 凭据命令的超时（规范：30 秒）。
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// 核心转发模型请求的目标：当前的提供方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsRoute {
    pub id: PluginId,
    pub spec: ModelsProvider,
    /// 核心访问网关接口的地址 `host:port`。
    pub upstream: String,
    /// 执行凭据命令的容器。
    pub credentials_container: String,
}

/// 凭据名 → 值。
type Keys = BTreeMap<String, String>;

/// 一个使用方的凭据，和最近一次成功注入的变量。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct KeyFile {
    keys: Keys,
    /// 最近一次成功注入给使用方的变量：之后重建容器时网关不在，也注入同一套值，网关回来就对上了。
    #[serde(default)]
    vars: Option<ModelVars>,
    /// 最近一次签发或验证失败的原因；接上了就清掉。详情页的轻提示带它（类别 [`wonton_protocol::PluginHintCategory::ModelsNotConnected`]）。
    #[serde(default)]
    problem: Option<String>,
    /// 默认路由最近一次探测的结论（见 `model_health`）：只在接上之后有意义，接上前的
    /// “没接上”由网关状态现场推导。恢复可用就清掉。
    #[serde(default)]
    route: Option<super::model_health::RouteProbe>,
}

/// 使用方的凭据，每个插件一个 JSON 文件（0600）。卸载时连同网关里的凭据一起删掉。
pub struct ModelKeyStore {
    pub(super) dir: PathBuf,
    /// 默认路由的探测跨安装、启动与巡检串行（见 `model_health`）：不发并发的计费请求。
    pub(super) probe_lock: tokio::sync::Mutex<()>,
    pub(super) probe: tokio::sync::Mutex<super::model_health::ProbeSchedule>,
}

impl KeyFile {
    /// 凭据签发过、也没有没签发完的问题：使用方已接上网关。
    fn connected(&self) -> bool {
        !self.keys.is_empty() && self.problem.is_none()
    }
}

impl ModelKeyStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            probe_lock: tokio::sync::Mutex::new(()),
            probe: tokio::sync::Mutex::default(),
        }
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn load(&self, id: &PluginId) -> Result<KeyFile, StoreError> {
        Ok(read_json::<KeyFile>(&self.path(id))?.unwrap_or_default())
    }

    /// 记下的一个凭据值：别的模块（建 Bot）只读。没有记过时是 `None`。
    pub(in crate::plugin) fn stored_key(&self, id: &PluginId, name: &str) -> Option<String> {
        self.load(id).ok()?.keys.get(name).cloned()
    }

    fn save(&self, id: &PluginId, file: &KeyFile) -> Result<(), StoreError> {
        create_private_dir(&self.dir)?;
        write_json(&self.path(id), file)
    }

    fn remove(&self, id: &PluginId) -> Result<(), StoreError> {
        remove_file(&self.path(id))
    }

    /// 记着凭据的使用方。
    fn all(&self) -> Result<Vec<(PluginId, KeyFile)>, StoreError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(StoreError::Io {
                    path: self.dir.clone(),
                    source,
                });
            }
        };
        let mut all = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| PluginId::parse(s).ok());
            if let Some(id) = id
                && path.extension().and_then(|e| e.to_str()) == Some("json")
            {
                let file = self.load(&id)?;
                all.push((id, file));
            }
        }
        Ok(all)
    }

    /// 详情页轻提示的依据：使用方的凭据是否就绪、记下的签发问题和路由探测结论。
    /// 组装在 `model_health`；KeyFile 的字段不外泄。
    pub(super) fn hint_basis(
        &self,
        id: &PluginId,
    ) -> (
        bool,
        Option<String>,
        Option<super::model_health::RouteProbe>,
    ) {
        match self.load(id) {
            Ok(file) => (!file.keys.is_empty(), file.problem, file.route),
            Err(_) => (false, None, None),
        }
    }

    /// 记下默认路由最近一次探测的结论（变了才写盘）；探测结果所有运行中的使用方共享。
    pub(super) fn update_route(
        &self,
        id: &PluginId,
        route: Option<super::model_health::RouteProbe>,
    ) {
        let Ok(mut file) = self.load(id) else {
            return;
        };
        if file.route == route {
            return;
        }
        file.route = route;
        if let Err(e) = self.save(id, &file) {
            tracing::warn!(plugin = %id, error = %e, "记下默认路由探测结论失败");
        }
    }
}

/// 使用方要的凭据名：没有 `bots` 时只有插件级的（插件 `id`）；有 `bots` 时 Bot 服务用 `<id>/<Bot 名>`，
/// 还有全插件一份的服务时再加插件级的。`bot_names` 是记录在案的 Bot 名。
fn credential_names(
    package: &PluginPackage,
    bot_names: &[String],
) -> (Option<String>, Vec<String>) {
    let manifest = &package.manifest;
    let id = manifest.id.to_string();
    let Some(bots_declared) = &manifest.bots else {
        return (Some(id), Vec::new());
    };
    let shared = package
        .service_names()
        .any(|service| !bots_declared.services.iter().any(|b| b == service));
    let bots = bot_names.iter().map(|bot| format!("{id}/{bot}")).collect();
    (shared.then(|| id.clone()), bots)
}

pub(super) fn uses_models(manifest: &Manifest) -> bool {
    manifest.permissions.contains(&Permission::Models)
}

/// 网关接口的基础地址：经核心在插件网络里的别名，不带末尾的 `/`。
fn base_url(port: u16, path: &str) -> String {
    format!(
        "http://{PLUGIN_MODELS_HOST}:{port}{}",
        path.trim_end_matches('/')
    )
}

/// 模型网关现在的状态。
pub enum ProviderState {
    Absent,
    /// 装了，但没在运行（安装中、异常、卸载中）。
    NotReady(PluginId),
    Ready(Box<ModelsRoute>),
}

pub(super) fn unavailable(message: impl Into<String>) -> InstallError {
    InstallError::ModelsUnavailable(message.into())
}

pub(crate) fn gateway_not_running(langs: &Languages, id: &str) -> String {
    crate::l10n::text(
        langs,
        "plugin-settings-models-gateway-not-running",
        &[("id", id.into())],
    )
}

pub(crate) fn gateway_absent(langs: &Languages) -> String {
    crate::l10n::text(langs, "plugin-settings-models-gateway-absent", &[])
}

/// 没接上网关时注入的变量：`WONTON_MODEL_*` 全部定义成空字符串，compose 引用它们不会因为变量缺失而失败，
/// 插件用它自己的登录或 key 设置。
fn empty_model_vars(package: &PluginPackage, bot_names: &[String]) -> ModelVars {
    let (plugin, _) = credential_names(package, bot_names);
    ModelVars {
        base_url: String::new(),
        anthropic_url: Some(String::new()),
        gemini_url: Some(String::new()),
        plugin_key: plugin.map(|_| String::new()),
        bot_keys: bot_names
            .iter()
            .map(|bot| (bot.clone(), String::new()))
            .collect(),
        default_model: String::new(),
    }
}

impl PluginManager {
    /// 当前可用的提供方：已安装、运行中的、声明了 `provides.models` 的插件。
    pub fn models_route(&self) -> Option<ModelsRoute> {
        match self.models_provider() {
            ProviderState::Ready(route) => Some(*route),
            _ => None,
        }
    }

    /// 模型设置页需要的网关网页和模型目录状态；目录由通用模型网关接口探测，不依赖插件私有命令。
    pub(crate) async fn model_settings(
        &self,
        langs: &Languages,
    ) -> Result<ModelSettings, InstallError> {
        let route = self.provider(langs)?;
        let (_, catalog) = self.model_probe_catalog(&route, langs).await?;
        let has_configured_models = catalog
            .get("data")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|models| !models.is_empty());
        Ok(ModelSettings {
            plugin_id: route.id.to_string(),
            web_url: self.model_web_url(&route.id),
            has_configured_models,
        })
    }

    /// 网关插件的网页地址，没有网页时为空。
    pub(crate) fn model_web_url(&self, id: &PluginId) -> Option<String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins.get(id).and_then(|entry| self.open_url(id, entry))
    }

    fn provider(&self, langs: &Languages) -> Result<ModelsRoute, InstallError> {
        match self.models_provider() {
            ProviderState::Ready(route) => Ok(*route),
            ProviderState::NotReady(id) => {
                Err(unavailable(gateway_not_running(langs, id.as_str())))
            }
            ProviderState::Absent => Err(unavailable(gateway_absent(langs))),
        }
    }

    /// 安装声明了 `models` 的插件时：网关就绪就签发并验证凭据；没装、没就绪或签发失败都照样装，
    /// 变量定义成空字符串（见 [`empty_model_vars`]）。签发失败记日志、原因记进凭据文件，
    /// 详情页的轻提示带上它；网关不在的情形详情页现场就能算出来，不记。
    pub(super) async fn provision_models(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<Option<ModelVars>, InstallError> {
        let manifest = &package.manifest;
        if !uses_models(manifest) {
            return Ok(None);
        }
        let bot_names = self.bot_names(&manifest.id);
        let vars = match self.provider(langs) {
            Err(e) => {
                tracing::info!(plugin = %manifest.id, error = %e, "模型网关没接上，照样安装，模型变量为空");
                empty_model_vars(package, &bot_names)
            }
            Ok(route) => match self
                .issue_consumer_credentials(&route, package, langs, &bot_names)
                .await
            {
                Ok(vars) => vars,
                Err(e) => {
                    tracing::warn!(plugin = %manifest.id, error = %e, "模型网关凭据签发失败，照样安装，模型变量为空");
                    empty_model_vars(package, &bot_names)
                }
            },
        };
        Ok(Some(vars))
    }

    /// 给使用方签发凭据并验证：缺的名字生成新值，逐个签发；成功后把变量一并记下（重建容器时用），
    /// 失败就吊销已经签发的、清掉记下的值，只留下原因。网关在签发中途坏掉时也照此办理，不留半套凭据。
    async fn issue_consumer_credentials(
        &self,
        route: &ModelsRoute,
        package: &PluginPackage,
        langs: &Languages,
        bot_names: &[String],
    ) -> Result<ModelVars, InstallError> {
        let store = self.store.model_keys();
        let id = &package.manifest.id;
        let mut file = store.load(id)?;
        let (plugin, bots) = credential_names(package, bot_names);
        for name in plugin.iter().chain(bots.iter()) {
            file.keys.entry(name.clone()).or_insert_with(random_token);
        }
        let issued = async {
            for name in plugin.iter().chain(bots.iter()) {
                self.issue_verified(route, name, &file.keys[name], langs)
                    .await?;
            }
            Ok::<_, InstallError>(())
        }
        .await;
        match issued {
            Ok(()) => {
                let vars = self.model_vars(route, package, &file.keys, bot_names);
                file.vars = Some(vars.clone());
                file.problem = None;
                // 凭据重新接上：之前的探测结论是旧状态留下的，下一轮探测重写。
                file.route = None;
                store.save(id, &file)?;
                Ok(vars)
            }
            Err(e) => {
                for name in file.keys.keys() {
                    let revoke = &route.spec.credentials.revoke;
                    if let Err(cleanup) = self.run_command(route, revoke, name, None, langs).await {
                        tracing::error!(plugin = %id, credential = %name, error = %cleanup, "吊销没签发完的模型网关凭据失败");
                    }
                }
                file.keys.clear();
                file.vars = None;
                file.problem = Some(e.to_string());
                store.save(id, &file)?;
                Err(e)
            }
        }
    }

    /// 升级时：旧凭据原样留着（新版本装不好还要退回去），新版本多要的名字网关就绪就补签（记进 `added`），
    /// 网关不在就沿用记下的变量（可能为空），缺的等网关回来在对账时补。
    pub(super) async fn upgrade_model_vars(
        &self,
        package: &PluginPackage,
        added: &mut Vec<String>,
        langs: &Languages,
    ) -> Result<Option<ModelVars>, InstallError> {
        let manifest = &package.manifest;
        if !uses_models(manifest) {
            return Ok(None);
        }
        let store = self.store.model_keys();
        let mut file = store.load(&manifest.id)?;
        let Ok(route) = self.provider(langs) else {
            return Ok(Some(file.vars.clone().unwrap_or_else(|| {
                empty_model_vars(package, &self.bot_names(&manifest.id))
            })));
        };
        let bot_names = self.bot_names(&manifest.id);
        let (plugin, bots) = credential_names(package, &bot_names);
        for name in plugin.iter().chain(bots.iter()) {
            if file.keys.contains_key(name) {
                continue;
            }
            let key = random_token();
            file.keys.insert(name.clone(), key.clone());
            added.push(name.clone());
            self.issue_verified(&route, name, &key, langs).await?;
        }
        let vars = self.model_vars(&route, package, &file.keys, &bot_names);
        file.vars = Some(vars.clone());
        file.problem = None;
        file.route = None;
        store.save(&manifest.id, &file)?;
        Ok(Some(vars))
    }

    /// 退回旧版本时吊销 [`Self::upgrade_model_vars`] 多签发的。网关已经卸载就没有可吊销的，只清掉记下的值；
    /// 网关装着但没在运行时吊销不了，如实报错。
    pub(super) async fn revoke_model_names(
        &self,
        id: &PluginId,
        names: &[String],
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if names.is_empty() {
            return Ok(());
        }
        let store = self.store.model_keys();
        let mut file = store.load(id)?;
        match self.models_provider() {
            ProviderState::Absent => {}
            ProviderState::NotReady(_) => {
                return Err(unavailable(crate::l10n::text(
                    langs,
                    "plugin-settings-models-revoke-not-running",
                    &[("id", id.as_str().into())],
                )));
            }
            ProviderState::Ready(route) => {
                for name in names {
                    let revoke = &route.spec.credentials.revoke;
                    self.run_command(&route, revoke, name, None, langs).await?;
                }
            }
        }
        for name in names {
            file.keys.remove(name);
        }
        if file.keys.is_empty() {
            store.remove(id)?;
        } else {
            store.save(id, &file)?;
        }
        Ok(())
    }

    /// 给一个 Bot 签发自己的模型网关凭据（`<插件 id>/<Bot 名>`），建 Bot 时用。插件没申请 `models` 时不动。
    pub(super) async fn issue_bot_credential(
        &self,
        package: &PluginPackage,
        bot: &str,
        langs: &Languages,
    ) -> Result<Option<String>, InstallError> {
        let manifest = &package.manifest;
        if !uses_models(manifest) || manifest.bots.is_none() {
            return Ok(None);
        }
        let route = self.provider(langs)?;
        let store = self.store.model_keys();
        let mut file = store.load(&manifest.id)?;
        let name = format!("{}/{bot}", manifest.id);
        let key = file
            .keys
            .entry(name.clone())
            .or_insert_with(random_token)
            .clone();
        store.save(&manifest.id, &file)?;
        self.issue_verified(&route, &name, &key, langs).await?;
        Ok(Some(key))
    }

    /// 删掉一个 Bot：吊销它的模型网关凭据，再从记下的值里删掉。没有记过就什么都不做。
    pub(super) async fn revoke_bot_credential(
        &self,
        id: &PluginId,
        bot: &str,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let name = format!("{id}/{bot}");
        if !self.store.model_keys().load(id)?.keys.contains_key(&name) {
            return Ok(());
        }
        self.revoke_model_names(id, &[name], langs).await
    }

    /// 重建容器时（见 [`super::lan`]、[`super::egress`]）：注入记下的变量，不问网关在不在——网关不在，
    /// 变量也保持原样，等网关回来对账后就接上了。从来没接上的注入空变量。
    pub(super) fn stored_model_vars(
        &self,
        package: &PluginPackage,
    ) -> Result<Option<ModelVars>, InstallError> {
        if !uses_models(&package.manifest) {
            return Ok(None);
        }
        let file = self.store.model_keys().load(&package.manifest.id)?;
        Ok(Some(file.vars.clone().unwrap_or_else(|| {
            empty_model_vars(package, &self.bot_names(&package.manifest.id))
        })))
    }

    /// 注入的变量；`keys` 里有使用方要的每个凭据。
    fn model_vars(
        &self,
        route: &ModelsRoute,
        package: &PluginPackage,
        keys: &Keys,
        bot_names: &[String],
    ) -> ModelVars {
        let (plugin, _) = credential_names(package, bot_names);
        let spec = &route.spec;
        let port = self.cfg.gateway_port;
        ModelVars {
            base_url: base_url(port, &spec.apis.openai),
            anthropic_url: spec.apis.anthropic.as_ref().map(|p| base_url(port, p)),
            gemini_url: spec.apis.gemini.as_ref().map(|p| base_url(port, p)),
            plugin_key: plugin.map(|name| keys[&name].clone()),
            bot_keys: bot_names
                .iter()
                .map(|bot| {
                    (
                        bot.clone(),
                        keys[&format!("{}/{}", package.manifest.id, bot)].clone(),
                    )
                })
                .collect(),
            default_model: spec.default_model.clone(),
        }
    }

    /// 卸载使用方或安装失败回滚：在网关里吊销它的凭据，再删掉记下的文件（没有凭据也删——里面可能只记着
    /// 签发失败的原因）。网关已经卸载了就没有可吊销的；网关装着但没在运行时吊销不了，如实报错。
    pub(super) async fn revoke_models(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let store = self.store.model_keys();
        let keys = store.load(id)?.keys;
        match self.models_provider() {
            ProviderState::Absent => {}
            ProviderState::NotReady(provider) => {
                if keys.is_empty() {
                    store.remove(id)?;
                    return Ok(());
                }
                return Err(unavailable(crate::l10n::text(
                    langs,
                    "plugin-settings-models-revoke-provider-not-running",
                    &[
                        ("provider", provider.as_str().into()),
                        ("id", id.as_str().into()),
                    ],
                )));
            }
            ProviderState::Ready(route) => {
                for name in keys.keys() {
                    let revoke = &route.spec.credentials.revoke;
                    self.run_command(&route, revoke, name, None, langs).await?;
                }
            }
        }
        store.remove(id)?;
        Ok(())
    }

    /// 网关启动后（安装、恢复、升级）为使用方对一次账：记着凭据的用原值重新签发并验证，对账失败的标为异常、
    /// 写明原因；没接上的（装它的时候网关不在或签发失败）现在签发，并按新变量重建运行中的容器。
    pub async fn reconcile_models(&self) -> Vec<(PluginId, InstallError)> {
        let langs = self.owner_languages();
        let mut failures = self.reconcile_model_keys(&langs).await;
        for (id, error) in &failures {
            let reason = crate::l10n::text(
                &langs,
                "plugin-settings-models-reconcile-failed",
                &[("error", error.to_string().into())],
            );
            self.set(id, PluginStatus::Degraded { reason }, None);
        }
        failures.extend(self.connect_unconnected_consumers(&langs).await);
        failures
    }

    async fn reconcile_model_keys(&self, langs: &Languages) -> Vec<(PluginId, InstallError)> {
        let Some(route) = self.models_route() else {
            return Vec::new();
        };
        let all = match self.store.model_keys().all() {
            Ok(all) => all,
            Err(e) => return vec![(route.id.clone(), e.into())],
        };
        let mut failures = Vec::new();
        for (id, file) in all {
            for (name, value) in &file.keys {
                if let Err(e) = self.issue_verified(&route, name, value, langs).await {
                    tracing::warn!(plugin = %id, credential = %name, error = %e, "模型网关凭据对账失败");
                    failures.push((id.clone(), e));
                }
            }
            // 网关升级可能改了接口路径和默认模型：对账顺手把记下的变量刷新成现在的。
            if !file.keys.is_empty()
                && let Ok(Some(record)) = self.store.load(&id)
                && let Ok(mut package) =
                    PluginPackage::with_files(&record.source, record.files, &self.owner_languages())
            {
                package.origin = record.origin.clone();
                let vars = self.model_vars(&route, &package, &file.keys, &self.bot_names(&id));
                let mut file = file;
                file.vars = Some(vars);
                file.problem = None;
                file.route = None;
                if let Err(e) = self.store.model_keys().save(&id, &file) {
                    tracing::warn!(plugin = %id, error = %e, "记下模型网关变量失败");
                }
            }
        }
        failures
    }

    /// 给没接上的使用方签发凭据：装它的时候网关不在或签发失败，变量是空的。签发成功后，运行中的按新变量
    /// 重建容器；没在运行的等下次启动时从记下的变量里拿。还接不上的记下原因，详情页的轻提示带它。
    async fn connect_unconnected_consumers(
        &self,
        langs: &Languages,
    ) -> Vec<(PluginId, InstallError)> {
        let Some(route) = self.models_route() else {
            return Vec::new();
        };
        let candidates: Vec<(PluginId, bool)> = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins
                .iter()
                .filter(|(id, entry)| {
                    **id != route.id
                        && entry.listed.as_ref().is_some_and(|listed| {
                            uses_models(&listed.manifest)
                                && matches!(
                                    entry.status,
                                    PluginStatus::Running | PluginStatus::Stopped
                                )
                        })
                })
                .map(|(id, entry)| (id.clone(), entry.status == PluginStatus::Running))
                // 只补没接上的：已接上的对账刚把凭据对好，变量没变，容器不用动。
                .filter(|(id, _)| {
                    !self
                        .store
                        .model_keys()
                        .load(id)
                        .is_ok_and(|file| file.connected())
                })
                .collect()
        };
        let mut failures = Vec::new();
        for (id, running) in candidates {
            let Ok(Some(record)) = self.store.load(&id) else {
                continue;
            };
            let Ok(mut package) = PluginPackage::with_files(
                &record.source,
                record.files.clone(),
                &self.owner_languages(),
            ) else {
                continue;
            };
            package.origin = record.origin.clone();
            let bot_names = self.bot_names(&id);
            match self
                .issue_consumer_credentials(&route, &package, langs, &bot_names)
                .await
            {
                Err(e) => {
                    tracing::warn!(plugin = %id, error = %e, "模型网关凭据签发失败，使用方保持没接上");
                    failures.push((id, e));
                }
                Ok(_) if running => match self.rebuild_for_models(&package, &record).await {
                    Ok(()) => tracing::info!(plugin = %id, "模型网关接上了，使用方已按新变量重建"),
                    Err(e) => {
                        tracing::error!(plugin = %id, error = %e, "模型网关接上了，重建使用方失败");
                        failures.push((id, e));
                    }
                },
                Ok(_) => {
                    tracing::info!(plugin = %id, "模型网关接上了，使用方没在运行，下次启动带上变量")
                }
            }
        }
        failures
    }

    /// 模型网关接上后重建使用方的容器：和安装时一样渲染，全部容器换一遍。
    async fn rebuild_for_models(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<(), InstallError> {
        let lan = self
            .lan_grant(package, record.lan_webui, &self.owner_languages())
            .await?;
        let (pending, reservation) = match self.prepare_migration(&package.manifest).await? {
            Some((pending, reservation)) => (Some(pending), Some(reservation)),
            None => (None, None),
        };
        let mut ctx = self
            .rebuild_context(package, record, lan, pending.as_ref())
            .await?;
        // Bot 的 Matrix 变量和总线地址也要填：Bot 服务的容器引用了它们，令牌从旧容器读回。
        self.fill_bot_vars(package, &mut ctx, &self.owner_languages())
            .await?;
        let plan = render::render(package, &ctx, &self.owner_languages())?;
        if let (Some(pending), Some(reservation)) = (&pending, reservation) {
            self.apply_migration(
                &package.manifest,
                pending,
                reservation,
                ctx.egress.is_some(),
                &self.owner_languages(),
            )
            .await?;
        }
        self.replace_containers(&ctx, plan, Gate::StartPeriod)
            .await?;
        self.reconcile_running(package, record).await?;
        Ok(())
    }

    /// 签发，再用这个凭据请求一次模型列表。返回 401 说明网关里的同名凭据不是这个值：先吊销再签发一次。
    async fn issue_verified(
        &self,
        route: &ModelsRoute,
        name: &str,
        value: &str,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        self.issue_verified_catalog(route, name, value, langs)
            .await
            .map(|_| ())
    }

    /// 在网关的容器里执行凭据命令，`{name}` 逐项替换成凭据名，不经 shell。
    pub(super) async fn run_command(
        &self,
        route: &ModelsRoute,
        argv: &[String],
        name: &str,
        value: Option<&str>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let argv: Vec<String> = argv
            .iter()
            .map(|arg| arg.replace(NAME_PLACEHOLDER, name))
            .collect();
        let stdin = value.map(|v| format!("{v}\n"));
        let output = self
            .engine
            .exec(
                &route.credentials_container,
                &argv,
                stdin.as_deref().map(str::as_bytes),
                COMMAND_TIMEOUT,
            )
            .await
            .map_err(|e| {
                unavailable(crate::l10n::text(
                    langs,
                    "plugin-settings-models-not-ready",
                    &[
                        ("provider", route.id.as_str().into()),
                        ("error", e.to_string().into()),
                    ],
                ))
            })?;
        if output.exit_code == 0 {
            return Ok(());
        }
        Err(unavailable(crate::l10n::text(
            langs,
            "plugin-settings-models-command-rejected",
            &[
                ("provider", route.id.as_str().into()),
                ("command", argv.first().map_or("", String::as_str).into()),
                ("code", output.exit_code.to_string().into()),
                ("stderr", output.stderr.clone().into()),
            ],
        )))
    }
}
