//! 只启动现存的常驻容器，不重建配置、不重跑已完成的一次性服务。

use std::collections::BTreeSet;
use std::sync::Arc;

use tokio::time::Instant;

use super::compose::DependCondition;
use super::health::{self, Target};
use super::health_gate::Gate;
use super::languages::Languages;
use super::manager::{
    Deployed, Entry, Health, InstallError, PluginManager, PluginStatus, PluginView, busy, detached,
    now_secs,
};
use super::manifest::PluginId;
use super::names::LABEL_PLUGIN;
use super::package::PluginPackage;
use super::reconcile::ContainersState;
use super::render::start_order;
use super::store::InstalledRecord;
use crate::l10n;

struct Started {
    health: Health,
    serving: BTreeSet<String>,
    deployed: Option<Deployed>,
}

impl PluginManager {
    /// 请求断开也完成启动；成功前检查容器就绪、网页入口及插件身份配置。
    /// 返回 `probe`:刚启动了模型使用方时由调用方补一轮一次性探测(见 model_health)。
    pub async fn start_detached(
        self: &Arc<Self>,
        id: PluginId,
    ) -> Result<PluginView, InstallError> {
        let this = Arc::clone(self);
        let started_id = id.clone();
        let closing = Arc::clone(&this);
        let (view, probe) = detached(async move { this.start(&started_id).await }).await?;
        if probe {
            closing.check_models_once(vec![id]).await;
        }
        Ok(view)
    }

    async fn start(&self, id: &PluginId) -> Result<(PluginView, bool), InstallError> {
        let langs = self.owner_languages();
        self.check_not_upgrading(&langs)?;
        let before = self.begin_start(id, &langs)?;
        let result = self.start_existing(id, &before, &langs).await;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins.get_mut(id).expect("启动期间其他操作不能删除插件");
        let result = match result {
            Ok(Started {
                health,
                serving,
                deployed,
            }) => {
                let started = deployed.is_some();
                if let Some(deployed) = deployed {
                    let listed = entry.listed.clone().expect("启动前检查了插件记录");
                    *entry = Entry::running(listed, deployed);
                } else {
                    entry.status = PluginStatus::Running;
                    entry.health = Some(health);
                    entry.serving = serving;
                    entry.checked_at = now_secs();
                }
                let probe = started
                    && entry
                        .listed
                        .as_ref()
                        .is_some_and(|l| super::model_health::needs_default_model(&l.manifest));
                Ok((self.view(id, entry), probe))
            }
            Err(error) => {
                entry.status = PluginStatus::Degraded {
                    reason: l10n::text(
                        &langs,
                        "plugin-install-start-failed",
                        &[("error", error.to_string().into())],
                    ),
                };
                entry.health = None;
                entry.follows_containers = true;
                entry.checked_at = now_secs();
                Err(error)
            }
        };
        drop(plugins);
        self.lan.end(id);
        self.changed();
        let (view, probe) = result?;
        Ok((view, probe))
    }

    fn begin_start(&self, id: &PluginId, langs: &Languages) -> Result<PluginStatus, InstallError> {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins
            .get_mut(id)
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        if let Some(error) = busy(id, &entry.status, langs) {
            return Err(error);
        }
        if self.lan.is_rebuilding(id) {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-rebuilding",
                &[("id", id.to_string().into())],
            )));
        }
        if let PluginStatus::Degraded { reason } = &entry.status
            && !entry.follows_containers
        {
            return Err(InstallError::Conflict(reason.clone()));
        }
        if entry.listed.is_none() {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-record-unusable",
                &[],
            )));
        }
        if !self.lan.begin(id) {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-rebuilding",
                &[("id", id.to_string().into())],
            )));
        }
        let before = entry.status.clone();
        Ok(before)
    }

    async fn start_existing(
        &self,
        id: &PluginId,
        before: &PluginStatus,
        langs: &Languages,
    ) -> Result<Started, InstallError> {
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        let mut package = PluginPackage::with_files(&record.source, record.files.clone(), langs)?;
        package.origin = record.origin.clone();
        let checked = self.check_containers(&package, langs).await?;
        let state = checked.ready(langs)?;
        if let ContainersState::Running(health) = &state
            && *before == PluginStatus::Running
        {
            return Ok(Started {
                health: *health,
                serving: checked.serving().clone(),
                deployed: None,
            });
        }
        // 没有渲染计划：等待时限从生效的 healthcheck（在容器配置里）算，和安装、升级同一条规则。
        let deadline = self.wait_deadline(&package).await?;
        if let ContainersState::Stopped = state {
            self.set(id, PluginStatus::Installing, None);
            // 复用恢复核对，先把核心接回网络；依赖核心内部别名的服务才能启动。
            self.reconcile(&package, &record, Gate::StartPeriod).await?;
            // 渲染变了就重建，免得主人启动的还是旧配置的容器。
            if !self
                .recreate_on_drift(&package, &record, langs, true, Gate::StartPeriod)
                .await?
            {
                self.start_resident_containers(&package, &record, deadline, langs)
                    .await?;
            }
        }
        let deployed = self.reconcile_running(&package, &record).await?;
        if let (Some(upstream), Some(webui)) = (&deployed.upstream, &package.manifest.webui) {
            health::wait_webui(&self.http, upstream, &webui.path, deadline, langs).await?;
        }
        Ok(Started {
            health: deployed.health,
            serving: BTreeSet::new(),
            deployed: Some(deployed),
        })
    }

    /// 启动不重渲染计划：生效的 healthcheck（安装时把镜像自带的并了进去）在容器配置里，等待时限据此算。
    async fn wait_deadline(&self, package: &PluginPackage) -> Result<Instant, InstallError> {
        let slug = package.manifest.id.slug();
        let bot_services = super::bot_services::bot_services(&package.manifest);
        let bots = self.bot_names(&package.manifest.id);
        let mut configs = Vec::new();
        for service in package.service_names() {
            let instances: Vec<String> = if bot_services.contains(service) && !bots.is_empty() {
                bots.iter()
                    .map(|bot| self.cfg.instance.bot_container(&slug, service, bot))
                    .collect()
            } else {
                vec![self.cfg.instance.container(&slug, service)]
            };
            for name in instances {
                let config = self
                    .engine
                    .inspect_container(&name)
                    .await?
                    .and_then(|info| info.config);
                configs.push(config.and_then(|config| config.healthcheck));
            }
        }
        Ok(super::health_gate::deadline_from_configs(
            configs.iter().map(Option::as_ref),
            self.cfg.install_timeout,
            Instant::now(),
        ))
    }

    async fn start_resident_containers(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        deadline: Instant,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let id = &package.manifest.id;
        let slug = id.slug();
        let dependencies = package.service_dependencies(langs)?;
        let order = start_order(
            dependencies
                .iter()
                .map(|(name, deps)| (*name, deps.iter().map(|(name, _)| name.clone()).collect()))
                .collect(),
            langs,
        )?;
        let one_shot = package.one_shot_services();
        let bot_services = super::bot_services::bot_services(&package.manifest);
        let bots = self.bot_names(&package.manifest.id);
        // 要启动的容器：Bot 服务的按 Bot 一组，其余全插件一份；启动顺序不变。
        let name_of = |service: &str, bot: Option<&str>| match bot {
            Some(bot) => self.cfg.instance.bot_container(&slug, service, bot),
            None => self.cfg.instance.container(&slug, service),
        };
        let mut group: Vec<(&str, Option<&str>)> = Vec::new();
        for service in &order {
            if one_shot.contains(service.as_str()) {
                continue;
            }
            if bot_services.contains(service.as_str()) && !bots.is_empty() {
                group.extend(
                    bots.iter()
                        .map(|bot| (service.as_str(), Some(bot.as_str()))),
                );
            } else {
                group.push((service.as_str(), None));
            }
        }
        // 启动前逐个确认归属：同名但被别的容器占用时不碰它。
        for (service, bot) in &group {
            let name = name_of(service, *bot);
            let info = self.engine.inspect_container(&name).await?.ok_or_else(|| {
                InstallError::Unhealthy(l10n::text(
                    langs,
                    "plugin-install-container-missing",
                    &[("name", name.as_str().into())],
                ))
            })?;
            let labels = info
                .config
                .and_then(|config| config.labels)
                .unwrap_or_default();
            if !self.cfg.instance.owns(&labels)
                || labels.get(LABEL_PLUGIN).map(String::as_str) != Some(id.as_str())
            {
                return Err(InstallError::Conflict(l10n::text(
                    langs,
                    "plugin-install-container-not-owned",
                    &[
                        ("name", name.as_str().into()),
                        ("id", id.to_string().into()),
                    ],
                )));
            }
        }
        self.register_source(&package.manifest, &self.cfg.instance.network(&slug))
            .await?;
        for (service, bot) in &group {
            // 一次性服务不重跑（见文件头）：它上次以 0 退出才算满足。
            let deps: Vec<(String, DependCondition)> = dependencies[service]
                .iter()
                .map(|(dependency, condition)| {
                    let condition = if one_shot.contains(dependency.as_str()) {
                        DependCondition::ServiceCompletedSuccessfully
                    } else {
                        *condition
                    };
                    // 依赖的 Bot 服务是同一个 Bot 的那份；全插件一份的服务就它自己。
                    let dep_bot = bot.filter(|_| bot_services.contains(dependency.as_str()));
                    (name_of(dependency, dep_bot), condition)
                })
                .collect();
            health::wait_for_all(&self.engine, &deps, deadline, langs).await?;
            let name = name_of(service, *bot);
            self.engine.start_container(&name).await?;
        }
        for (service, bot) in &group {
            let name = name_of(service, *bot);
            health::wait_for(&self.engine, &name, Target::Ready, deadline, langs).await?;
        }
        self.ensure_relays(&package.manifest, record).await?;
        Ok(())
    }
}
