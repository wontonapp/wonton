use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{
    Languages, Manifest, ModelVars, Permission, PluginId, PluginPackage, StoreError,
    create_private_dir, read_json, remove_file, write_json,
};

/// 凭据名 → 值。
pub(super) type Keys = BTreeMap<String, String>;

/// 一个使用方的凭据，和最近一次成功注入的变量。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::plugin) struct KeyFile {
    pub(super) keys: Keys,
    /// 最近一次成功注入给使用方的变量：之后重建容器时网关不在，也注入同一套值，网关回来就对上了。
    #[serde(default)]
    pub(super) vars: Option<ModelVars>,
    /// 最近一次签发或验证失败的原因；接上了就清掉。详情页的轻提示带它（类别 [`wonton_protocol::PluginHintCategory::ModelsNotConnected`]）。
    #[serde(default)]
    pub(super) problem: Option<String>,
    /// 最近一次对账（用记下的值重新签发并验证）失败的原因。凭据值还在，巡检按它重试，成功就清掉，
    /// 别让一次超时把使用方永远标成异常。
    #[serde(default)]
    pub(super) reconcile_problem: Option<String>,
    /// 默认路由最近一次探测的结论（见 `model_health`）：只在接上之后有意义，接上前的
    /// “没接上”由网关状态现场推导。恢复可用就清掉。
    #[serde(default)]
    pub(super) route: Option<super::model_health::RouteProbe>,
    /// 使用方卸载时网关没在运行，凭据没吊销成；网关回来后对账补吊销，成功了再删掉这个文件。
    #[serde(default)]
    pub(in crate::plugin) revoke_pending: bool,
}

/// 还没对上结局的凭据写命令：exec、容器和使用方。落盘，重启或断连后靠它判断。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PendingCommand {
    pub(super) exec_id: String,
    pub(super) container_id: String,
    pub(super) consumer: String,
}

/// 使用方的凭据，每个插件一个 JSON 文件（0600）。卸载时连同网关里的凭据一起删掉。
pub struct ModelKeyStore {
    pub(in crate::plugin) dir: PathBuf,
    /// 默认路由的探测跨安装、启动与巡检串行（见 `model_health`）：不发并发的计费请求。
    pub(in crate::plugin) probe_lock: tokio::sync::Mutex<()>,
    pub(in crate::plugin) probe: tokio::sync::Mutex<super::model_health::ProbeSchedule>,
    /// 凭据命令串行：确认上一条的结局后才发下一条（见 `run_command`）。
    pub(super) commands: tokio::sync::Mutex<()>,
}

impl KeyFile {
    /// 凭据签发过、也没有没签发完的问题：使用方已接上网关。
    pub(super) fn connected(&self) -> bool {
        !self.keys.is_empty() && self.problem.is_none()
    }
}

impl ModelKeyStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            probe_lock: tokio::sync::Mutex::new(()),
            probe: tokio::sync::Mutex::default(),
            commands: tokio::sync::Mutex::new(()),
        }
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub(in crate::plugin) fn load(&self, id: &PluginId) -> Result<KeyFile, StoreError> {
        Ok(read_json::<KeyFile>(&self.path(id))?.unwrap_or_default())
    }

    /// 记下的一个凭据值：别的模块（建 Bot）只读。没有记过时是 `None`。
    pub(in crate::plugin) fn stored_key(&self, id: &PluginId, name: &str) -> Option<String> {
        self.load(id).ok()?.keys.get(name).cloned()
    }

    pub(super) fn save(&self, id: &PluginId, file: &KeyFile) -> Result<(), StoreError> {
        create_private_dir(&self.dir)?;
        write_json(&self.path(id), file)
    }

    pub(super) fn remove(&self, id: &PluginId) -> Result<(), StoreError> {
        remove_file(&self.path(id))
    }

    fn pending_path(&self) -> PathBuf {
        self.dir.join("commands").join("pending.json")
    }

    pub(super) fn pending(&self) -> Result<Option<PendingCommand>, StoreError> {
        read_json(&self.pending_path())
    }

    pub(super) fn save_pending(&self, pending: &PendingCommand) -> Result<(), StoreError> {
        create_private_dir(&self.dir.join("commands"))?;
        write_json(&self.pending_path(), pending)
    }

    pub(super) fn clear_pending(&self) -> Result<(), StoreError> {
        remove_file(&self.pending_path())
    }

    /// 记着凭据的使用方。
    pub(in crate::plugin) fn all(&self) -> Result<Vec<(PluginId, KeyFile)>, StoreError> {
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

    /// 曾为至少一个使用方成功签发并验证过网关凭据。
    pub(super) fn has_connected_consumer(&self) -> Result<bool, StoreError> {
        Ok(self.all()?.into_iter().any(|(_, file)| file.connected()))
    }

    /// 详情页轻提示的依据：使用方的凭据是否就绪、记下的签发问题和路由探测结论。
    /// 组装在 `model_health`；KeyFile 的字段不外泄。`langs` 用来现场写“结果未定”的说明。
    pub(in crate::plugin) fn hint_basis(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> (
        bool,
        Option<String>,
        Option<super::model_health::RouteProbe>,
    ) {
        match (self.load(id), self.pending()) {
            (Ok(file), Ok(pending)) => {
                let pending_problem = pending
                    .filter(|command| command.consumer == id.as_str())
                    .map(|_| {
                        crate::l10n::text(
                            langs,
                            "plugin-settings-models-command-outcome-unknown",
                            &[],
                        )
                    });
                (
                    !file.keys.is_empty(),
                    file.problem.or(pending_problem),
                    file.route,
                )
            }
            (Err(error), _) | (_, Err(error)) => (false, Some(error.to_string()), None),
        }
    }

    /// 记下默认路由最近一次探测的结论（变了才写盘）；探测结果所有运行中的使用方共享。
    pub(in crate::plugin) fn update_route(
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
pub(super) fn credential_names(
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

/// 用了模型网关：能力依赖里写了 `models`（规范只允许放 `wants`），或老清单的 `permissions: [models]`。
pub(in crate::plugin) fn uses_models(manifest: &Manifest) -> bool {
    manifest
        .wants
        .iter()
        .chain(&manifest.requires)
        .any(|name| name == super::capabilities::MODELS.name)
        || manifest.permissions.contains(&Permission::Models)
}
