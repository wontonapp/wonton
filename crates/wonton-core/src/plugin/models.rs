//! 模型网关（插件规范的“模型网关”）。提供方是声明了 `provides.models` 的系统插件，同一时间只有一个；声明了
//! `models` 的插件经核心在插件网络里的别名 `models.wonton.internal` 访问它，各自拿一个只属于自己的凭据。
//!
//! 凭据按 Docker 凭据助手的约定签发：核心在网关的容器里执行声明的命令，值走标准输入，只看退出码；签发后用这个
//! 凭据请求一次 `GET <openai>/models`，返回 200 才算成功。值存在插件记录目录下的 `model-keys/`，网关的数据丢了或
//! 重装了，核心用同一个值重新签发。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use hyper::{Method, StatusCode};
use serde::{Deserialize, Serialize};

use super::bot_services::DEFAULT_BOT;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::{Manifest, Permission, PluginId};
use super::package::PluginPackage;
use super::provides::{ModelsProvider, NAME_PLACEHOLDER};
use super::render::ModelVars;
use super::store::{StoreError, create_private_dir, read_json, remove_file, write_json};
use crate::config::PLUGIN_MODELS_HOST;
use crate::http::RequestBody;
use crate::secret::random_token;

/// 凭据命令的超时（规范：30 秒）。
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
/// 验证凭据的请求超时。
const VERIFY_TIMEOUT: Duration = Duration::from_secs(30);

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

/// 一个使用方的凭据。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct KeyFile {
    keys: Keys,
}

/// 使用方的凭据，每个插件一个 JSON 文件（0600）。卸载时连同网关里的凭据一起删掉。
pub struct ModelKeyStore {
    dir: PathBuf,
}

impl ModelKeyStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn load(&self, id: &PluginId) -> Result<Keys, StoreError> {
        Ok(read_json::<KeyFile>(&self.path(id))?
            .unwrap_or_default()
            .keys)
    }

    fn save(&self, id: &PluginId, keys: &Keys) -> Result<(), StoreError> {
        create_private_dir(&self.dir)?;
        let file = KeyFile { keys: keys.clone() };
        write_json(&self.path(id), &file)
    }

    fn remove(&self, id: &PluginId) -> Result<(), StoreError> {
        remove_file(&self.path(id))
    }

    /// 有凭据的使用方。
    fn all(&self) -> Result<Vec<(PluginId, Keys)>, StoreError> {
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
                let keys = self.load(&id)?;
                all.push((id, keys));
            }
        }
        Ok(all)
    }
}

/// 使用方要的凭据名：没有 `bots` 时只有插件级的（插件 `id`）；有 `bots` 时 Bot 服务用 `<id>/<Bot 名>`，还有
/// 全插件一份的服务时再加插件级的。
fn credential_names(package: &PluginPackage) -> (Option<String>, Option<String>) {
    let manifest = &package.manifest;
    let id = manifest.id.to_string();
    let Some(bots) = &manifest.bots else {
        return (Some(id), None);
    };
    let shared = package
        .service_names()
        .any(|service| !bots.services.iter().any(|b| b == service));
    (
        shared.then(|| id.clone()),
        Some(format!("{id}/{DEFAULT_BOT}")),
    )
}

fn uses_models(manifest: &Manifest) -> bool {
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

fn unavailable(message: impl Into<String>) -> InstallError {
    InstallError::ModelsUnavailable(message.into())
}

impl PluginManager {
    /// 当前可用的提供方：已安装、运行中的、声明了 `provides.models` 的插件。
    pub fn models_route(&self) -> Option<ModelsRoute> {
        match self.models_provider() {
            ProviderState::Ready(route) => Some(*route),
            _ => None,
        }
    }

    fn provider(&self) -> Result<ModelsRoute, InstallError> {
        match self.models_provider() {
            ProviderState::Ready(route) => Ok(*route),
            ProviderState::NotReady(id) => Err(unavailable(format!("模型网关 {id} 没在运行"))),
            ProviderState::Absent => Err(unavailable(
                "没有装模型网关（声明 provides.models 的系统插件）",
            )),
        }
    }

    /// 安装声明了 `models` 的插件时：签发并验证凭据，返回要注入的变量。网关没装或没就绪时安装失败。
    pub(super) async fn provision_models(
        &self,
        package: &PluginPackage,
    ) -> Result<Option<ModelVars>, InstallError> {
        let manifest = &package.manifest;
        if !uses_models(manifest) {
            return Ok(None);
        }
        let route = self.provider()?;
        let store = self.store.model_keys();
        let mut keys = store.load(&manifest.id)?;
        let (plugin, bot) = credential_names(package);
        for name in plugin.iter().chain(bot.iter()) {
            keys.entry(name.clone()).or_insert_with(random_token);
        }
        store.save(&manifest.id, &keys)?;
        let issued = async {
            for name in plugin.iter().chain(bot.iter()) {
                self.issue_verified(&route, name, &keys[name]).await?;
            }
            Ok::<_, InstallError>(())
        }
        .await;
        if let Err(e) = issued {
            // 没签发完：已经签发的吊销掉，记下的值也删掉，不留半套凭据。
            if let Err(cleanup) = self.revoke_models(&manifest.id).await {
                tracing::error!(plugin = %manifest.id, error = %cleanup, "吊销没签发完的模型网关凭据失败");
            }
            return Err(e);
        }
        Ok(Some(self.model_vars(&route, package, &keys)))
    }

    /// 升级时：旧凭据原样留着（新版本装不好还要退回去），只签发新版本多要的，记进 `added`。
    pub(super) async fn upgrade_model_vars(
        &self,
        package: &PluginPackage,
        added: &mut Vec<String>,
    ) -> Result<Option<ModelVars>, InstallError> {
        let manifest = &package.manifest;
        if !uses_models(manifest) {
            return Ok(None);
        }
        let route = self.provider()?;
        let store = self.store.model_keys();
        let mut keys = store.load(&manifest.id)?;
        let (plugin, bot) = credential_names(package);
        for name in plugin.iter().chain(bot.iter()) {
            if keys.contains_key(name) {
                continue;
            }
            let key = random_token();
            keys.insert(name.clone(), key.clone());
            store.save(&manifest.id, &keys)?;
            added.push(name.clone());
            self.issue_verified(&route, name, &key).await?;
        }
        Ok(Some(self.model_vars(&route, package, &keys)))
    }

    /// 退回旧版本时吊销 [`Self::upgrade_model_vars`] 多签发的。
    pub(super) async fn revoke_model_names(
        &self,
        id: &PluginId,
        names: &[String],
    ) -> Result<(), InstallError> {
        if names.is_empty() {
            return Ok(());
        }
        let route = self.provider()?;
        let store = self.store.model_keys();
        let mut keys = store.load(id)?;
        for name in names {
            self.run_command(&route, &route.spec.credentials.revoke, name, None)
                .await?;
            keys.remove(name);
        }
        if keys.is_empty() {
            store.remove(id)?;
        } else {
            store.save(id, &keys)?;
        }
        Ok(())
    }

    /// 重建容器时（见 [`super::lan`]）：用记下的凭据，不重新签发。网关没在运行时如实报错。
    pub(super) fn stored_model_vars(
        &self,
        package: &PluginPackage,
    ) -> Result<Option<ModelVars>, InstallError> {
        if !uses_models(&package.manifest) {
            return Ok(None);
        }
        let route = self.provider()?;
        let keys = self.store.model_keys().load(&package.manifest.id)?;
        let (plugin, bot) = credential_names(package);
        if let Some(missing) = plugin
            .iter()
            .chain(bot.iter())
            .find(|n| !keys.contains_key(*n))
        {
            return Err(unavailable(format!("没有记下模型网关凭据 {missing}")));
        }
        Ok(Some(self.model_vars(&route, package, &keys)))
    }

    /// 注入的变量；`keys` 里有使用方要的每个凭据。
    fn model_vars(&self, route: &ModelsRoute, package: &PluginPackage, keys: &Keys) -> ModelVars {
        let (plugin, bot) = credential_names(package);
        let spec = &route.spec;
        let port = self.cfg.gateway_port;
        ModelVars {
            base_url: base_url(port, &spec.apis.openai),
            anthropic_url: spec.apis.anthropic.as_ref().map(|p| base_url(port, p)),
            gemini_url: spec.apis.gemini.as_ref().map(|p| base_url(port, p)),
            plugin_key: plugin.map(|name| keys[&name].clone()),
            bot_key: bot.map(|name| keys[&name].clone()),
            default_model: spec.default_model.clone(),
        }
    }

    /// 卸载使用方或安装失败回滚：在网关里吊销它的凭据，再删掉记下的值。网关已经卸载了就没有可吊销的；网关装着
    /// 但没在运行时吊销不了，如实报错。
    pub(super) async fn revoke_models(&self, id: &PluginId) -> Result<(), InstallError> {
        let store = self.store.model_keys();
        let keys = store.load(id)?;
        if keys.is_empty() {
            return Ok(());
        }
        match self.models_provider() {
            ProviderState::Absent => {}
            ProviderState::NotReady(provider) => {
                return Err(unavailable(format!(
                    "模型网关 {provider} 没在运行，吊销不了 {id} 的凭据"
                )));
            }
            ProviderState::Ready(route) => {
                for name in keys.keys() {
                    let revoke = &route.spec.credentials.revoke;
                    self.run_command(&route, revoke, name, None).await?;
                }
            }
        }
        store.remove(id)?;
        Ok(())
    }

    /// 网关启动后（安装、核心重启后的恢复）为所有使用方对一次账：用记下的值重新签发并验证。对账失败的使用方
    /// 标为异常、写明原因，不影响其他使用方。
    pub async fn reconcile_models(&self) -> Vec<(PluginId, InstallError)> {
        let failures = self.reconcile_model_keys().await;
        for (id, error) in &failures {
            let reason = format!("模型网关凭据对账失败：{error}");
            self.set(id, PluginStatus::Degraded { reason }, None);
        }
        failures
    }

    async fn reconcile_model_keys(&self) -> Vec<(PluginId, InstallError)> {
        let Some(route) = self.models_route() else {
            return Vec::new();
        };
        let all = match self.store.model_keys().all() {
            Ok(all) => all,
            Err(e) => return vec![(route.id.clone(), e.into())],
        };
        let mut failures = Vec::new();
        for (id, keys) in all {
            for (name, value) in &keys {
                if let Err(e) = self.issue_verified(&route, name, value).await {
                    tracing::warn!(plugin = %id, credential = %name, error = %e, "模型网关凭据对账失败");
                    failures.push((id.clone(), e));
                }
            }
        }
        failures
    }

    /// 签发，再用这个凭据请求一次模型列表。返回 401 说明网关里的同名凭据不是这个值：先吊销再签发一次。
    async fn issue_verified(
        &self,
        route: &ModelsRoute,
        name: &str,
        value: &str,
    ) -> Result<(), InstallError> {
        let issue = &route.spec.credentials.issue;
        self.run_command(route, issue, name, Some(value)).await?;
        if self.verify(route, value).await? {
            return Ok(());
        }
        let revoke = &route.spec.credentials.revoke;
        self.run_command(route, revoke, name, None).await?;
        self.run_command(route, issue, name, Some(value)).await?;
        if self.verify(route, value).await? {
            return Ok(());
        }
        Err(unavailable(format!(
            "模型网关 {} 不认核心签发的凭据 {name}：签发后请求模型列表仍然 401",
            route.id
        )))
    }

    /// 用凭据请求 `GET <openai>/models`：200 是认，401 是不认，其他算网关出错。
    async fn verify(&self, route: &ModelsRoute, value: &str) -> Result<bool, InstallError> {
        let path = route.spec.apis.openai.trim_end_matches('/');
        let url = format!("http://{}{path}/models", route.upstream);
        let (status, _) = self
            .http
            .request_json(
                Method::GET,
                &url,
                Some(value),
                RequestBody::Empty,
                VERIFY_TIMEOUT,
            )
            .await
            .map_err(|e| unavailable(format!("模型网关 {} 不可达：{e}", route.id)))?;
        match status {
            StatusCode::OK => Ok(true),
            StatusCode::UNAUTHORIZED => Ok(false),
            other => Err(unavailable(format!(
                "模型网关 {} 回答模型列表时出错：HTTP {other}",
                route.id
            ))),
        }
    }

    /// 在网关的容器里执行凭据命令，`{name}` 逐项替换成凭据名，不经 shell。
    async fn run_command(
        &self,
        route: &ModelsRoute,
        argv: &[String],
        name: &str,
        value: Option<&str>,
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
            .map_err(|e| unavailable(format!("模型网关 {} 未就绪：{e}", route.id)))?;
        if output.exit_code == 0 {
            return Ok(());
        }
        Err(unavailable(format!(
            "模型网关 {} 拒绝了凭据命令（{}，退出码 {}）：{}",
            route.id,
            argv.first().map_or("", String::as_str),
            output.exit_code,
            output.stderr
        )))
    }
}
