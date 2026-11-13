//! 建和删一个 Bot（ADR-0045）：每个 Bot 一组容器（`bots.services` 里的服务），自己的 Matrix 身份、模型
//! 凭据和总线令牌。只动这个 Bot 的容器，别的 Bot 不重启。给后续任务用的入口：家人加入时建私人 Bot、
//! 移除家人时删他的 Bot、Bot 的名单变了重建它的服务。

use std::collections::BTreeMap;
use std::fs;

use wonton_protocol::{BotKeeper, BotRef, NewSharedBot};

use super::bot_services::BotVars;
use super::bots::{BotEntry, check_bot_name, is_bot};
use super::health_gate;
use super::lifecycle::Undo;
use super::manager::{BotReport, InstallError, PluginManager, PluginStatus};
use super::manifest::{Manifest, SecretSpec};
use super::manifest::{Permission, PluginId};
use super::package::PluginPackage;
use super::render::{InstallContext, render};
use super::store::InstalledRecord;
use super::upgrade::installed_package;
use crate::config::PLUGIN_MCP_HOST;
use crate::plugin::languages::Languages;
use wonton_protocol::InstallStage;

/// 建好的 Bot。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BotCreated {
    pub bot: String,
    pub user_id: String,
    pub home_room: String,
}

/// 一个 Bot 的总线地址：令牌在路径里，Agent 的配置不用改（每个 MCP 客户端都支持 URL）。
fn mcp_url(port: u16, token: &str) -> String {
    format!("http://{PLUGIN_MCP_HOST}:{port}/mcp/{token}")
}
use crate::matrix::ProvisionedBot;

impl PluginManager {
    /// 给插件加一个 Bot。私人 Bot 的归属人在 `keeper` 里；共享 Bot 是 [`BotKeeper::Household`]。
    /// 插件没有声明 `bots` 时回明确的错误，不退回成单 Bot。
    pub async fn add_bot(
        &self,
        plugin: &PluginId,
        bot: &str,
        keeper: BotKeeper,
    ) -> Result<BotCreated, InstallError> {
        check_bot_name(bot).map_err(InstallError::Invalid)?;
        if bot == super::bot_services::DEFAULT_BOT {
            return Err(InstallError::Invalid(format!(
                "{bot} 是安装时默认 Bot 的名字，换一个"
            )));
        }
        let record = self
            .store
            .load(plugin)?
            .ok_or_else(|| InstallError::NotInstalled(plugin.to_string()))?;
        if let Some(status) = self.status_of(plugin)
            && let Some(busy) = super::manager::busy(plugin, &status, &self.owner_languages())
        {
            return Err(busy);
        }
        let package = installed_package(&record, &self.owner_languages())?;
        let manifest = &package.manifest;
        if manifest.bots.is_none() || manifest.kind != super::manifest::PluginKind::Agent {
            return Err(InstallError::Invalid(format!(
                "插件 {plugin} 没有声明 bots，不支持多个 Bot"
            )));
        }
        // 建 Bot 要启动它的一组容器，还会等全插件一份的服务健康：停止的插件先启动。
        if self.status_of(plugin).as_ref() != Some(&PluginStatus::Running) {
            return Err(InstallError::Invalid(format!(
                "插件 {plugin} 没在运行，先启动再建 Bot"
            )));
        }
        // 记录和 Matrix 用户名的分配都要独占：同时建的两个 Bot 不能互相覆盖。
        let _allocating = self.bot_allocation.lock().await;
        let langs = self.owner_languages();
        if self.store.bots().load(plugin)?.bots.contains_key(bot) {
            return Err(InstallError::Conflict(format!(
                "插件 {plugin} 已经有 Bot {bot}"
            )));
        }
        let provisioned = self
            .provision_bot_locked(plugin, manifest, bot, keeper.clone(), &package)
            .await?;
        let model_key = self.issue_bot_credential(&package, bot, &langs).await?;
        let plugin_bots = self.store.bots().load(plugin)?;
        let bus_token = plugin_bots
            .bots
            .get(bot)
            .map(|e| e.bus_token.clone())
            .unwrap_or_default();
        let runtime = BotVars {
            model_key,
            matrix: Some(self.matrix_vars(&provisioned)),
            mcp_url: Some(mcp_url(self.cfg.gateway_port, &bus_token)),
        };
        self.start_bot_containers(&package, &record, bot, runtime)
            .await?;
        let owner = self.keeper_user(&keeper);
        if self.hand_over_home_room(&owner, &provisioned).await? {
            tracing::info!(plugin = %plugin, bot, owner = %owner, "Bot 的主房间已交给归属人");
        }
        Ok(BotCreated {
            bot: bot.to_owned(),
            user_id: provisioned.account.user_id,
            home_room: provisioned.account.home_room,
        })
    }

    /// 删掉一个 Bot：删它的容器，吊销它的模型网关凭据和 Matrix 设备；`keep_data` 为假时停用账号（不可
    /// 恢复）、删掉它的数据目录。它的收存密钥（`collect` 记成 `名字@Bot` 的）一并清掉。别的 Bot 的容器不动。
    pub async fn remove_bot(
        &self,
        plugin: &PluginId,
        bot: &str,
        keep_data: bool,
    ) -> Result<BotReport, InstallError> {
        let record = self
            .store
            .load(plugin)?
            .ok_or_else(|| InstallError::NotInstalled(plugin.to_string()))?;
        if let Some(status) = self.status_of(plugin)
            && let Some(busy) = super::manager::busy(plugin, &status, &self.owner_languages())
        {
            return Err(busy);
        }
        let _allocating = self.bot_allocation.lock().await;
        let mut plugin_bots = self.store.bots().load(plugin)?;
        let Some(entry) = plugin_bots.bots.get(bot).cloned() else {
            return Err(InstallError::NotFound(format!(
                "插件 {plugin} 没有 Bot {bot}"
            )));
        };
        // 先删容器，再吊销凭据和设备。
        for name in self.bot_containers(plugin, bot).await? {
            self.engine.remove_container(&name).await?;
        }
        self.revoke_bot_credential(plugin, bot, &self.owner_languages())
            .await?;
        let matrix = self.matrix()?;
        let user = &entry.account.user_id;
        let devices = matrix.devices(user).await?;
        for device in &devices {
            matrix.delete_device(user, device).await?;
        }
        let mut deactivated = false;
        if !keep_data {
            matrix.deactivate(user).await?;
            deactivated = true;
        }
        self.forget_handoff_bot(plugin, bot)?;
        plugin_bots.bots.remove(bot);
        self.store.bots().save(plugin, &plugin_bots)?;
        self.tool_policy().forget_bot(plugin.as_str(), bot);
        let manifest = installed_package(&record, &self.owner_languages())?.manifest;
        self.drop_bot_secrets(plugin, &manifest, bot)?;
        if !keep_data {
            let dir = self.cfg.apps_dir.join(plugin.slug()).join("bots").join(bot);
            match fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(InstallError::Storage(format!(
                        "删除 {} 失败：{e}",
                        dir.display()
                    )));
                }
            }
        }
        tracing::info!(plugin = %plugin, bot, "Bot 已删除");
        Ok(BotReport {
            user_id: user.clone(),
            devices_removed: devices.len(),
            deactivated,
        })
    }

    /// 删掉这个 Bot 收存的密钥（`collect.service` 是 Bot 服务时记成 `名字@Bot` 的那些）。
    fn drop_bot_secrets(
        &self,
        plugin: &PluginId,
        manifest: &Manifest,
        bot: &str,
    ) -> Result<(), InstallError> {
        let bot_services: std::collections::BTreeSet<&str> = manifest
            .bots
            .iter()
            .flat_map(|b| &b.services)
            .map(String::as_str)
            .collect();
        let mut secrets = self.store.secrets().load(plugin)?.unwrap_or_default();
        let mut changed = false;
        for (name, spec) in &manifest.secrets {
            let SecretSpec::Collect(spec) = spec else {
                continue;
            };
            if bot_services.contains(spec.service.as_str())
                && secrets
                    .remove(super::secrets::collect_key(name, bot).as_str())
                    .is_some()
            {
                changed = true;
            }
        }
        if changed {
            self.store.secrets().save(plugin, &secrets)?;
        }
        Ok(())
    }

    /// 主人建一个共享 Bot：归全家，由主人管。主房间先交给主人，拉进家庭群是后续任务。
    pub async fn create_shared_bot(&self, req: &NewSharedBot) -> Result<BotRef, InstallError> {
        let plugin =
            PluginId::parse(&req.plugin_id).map_err(|e| InstallError::Invalid(e.to_string()))?;
        let record = self
            .store
            .load(&plugin)?
            .ok_or_else(|| InstallError::NotInstalled(plugin.to_string()))?;
        let package = installed_package(&record, &self.owner_languages())?;
        let manifest = &package.manifest;
        if manifest.bots.is_none() || manifest.kind != super::manifest::PluginKind::Agent {
            return Err(InstallError::Invalid(format!(
                "插件 {plugin} 没有声明 bots，不支持多个 Bot"
            )));
        }
        // storage.home 挂的是归属人的私人空间；共享 Bot 归全家，没有私人空间。
        if manifest
            .permissions
            .iter()
            .any(|p| matches!(p, Permission::StorageHome { .. }))
        {
            return Err(InstallError::Invalid(
                "这个插件申请了 storage.home（挂私人空间），不能建成共享 Bot：私人 Bot 是归属人的，\
                 共享 Bot 归全家、没有私人空间"
                    .to_owned(),
            ));
        }
        self.add_bot(&plugin, &req.bot, BotKeeper::Household)
            .await?;
        Ok(BotRef {
            plugin_id: plugin.to_string(),
            bot: req.bot.clone(),
        })
    }

    /// 名单变了（家人加入、联系人批了）：重建这个 Bot 的服务，沿用设备和令牌。
    pub(in crate::plugin) async fn rebuild_bot(
        &self,
        plugin: &PluginId,
        bot: &str,
    ) -> Result<(), InstallError> {
        let record = self
            .store
            .load(plugin)?
            .ok_or_else(|| InstallError::NotInstalled(plugin.to_string()))?;
        let package = installed_package(&record, &self.owner_languages())?;
        let base = {
            let lan = self
                .lan_grant(&package, record.lan_webui, &self.owner_languages())
                .await?;
            self.rebuild_context(&package, &record, lan, None).await?
        };
        let runtimes = self
            .bot_runtimes_read_back(&package, &base, &self.owner_languages())
            .await?;
        let runtime = runtimes
            .get(bot)
            .ok_or_else(|| InstallError::NotFound(format!("插件 {plugin} 没有 Bot {bot}")))?
            .clone();
        self.replace_bot_containers(&package, &record, bot, runtime)
            .await
    }

    /// Bot 插件的 Matrix 变量和总线地址：令牌从还在运行的旧容器读回（容器还没动），按 Bot 填进上下文。
    pub(in crate::plugin) async fn fill_bot_vars(
        &self,
        package: &PluginPackage,
        ctx: &mut InstallContext,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let manifest = &package.manifest;
        if !is_bot(manifest) {
            return Ok(());
        }
        let runtimes = self.bot_runtimes_read_back(package, ctx, langs).await?;
        if manifest.bots.is_some() {
            ctx.bots = runtimes;
        } else if let Some(runtime) = runtimes.get(super::bot_services::DEFAULT_BOT) {
            // 单个 Bot 的插件：身份在插件级变量里。旧版本没有服务引用令牌时读不回来，报错不让容器
            // 拿着空令牌起来。
            let matrix = runtime.matrix.clone().ok_or_else(|| {
                super::egress::cannot_read(
                    langs,
                    crate::l10n::text(langs, "plugin-settings-egress-token-not-used", &[]),
                )
            })?;
            if matrix.access_token.is_empty() {
                return Err(super::egress::cannot_read(
                    langs,
                    crate::l10n::text(langs, "plugin-settings-egress-token-not-used", &[]),
                ));
            }
            // 默认头像只补不覆盖（ADR-0044）：升级换包后补上没设置过的。
            self.fill_default_avatars(package, &matrix.user_id).await?;
            ctx.matrix = Some(matrix);
            ctx.mcp_url = runtime.mcp_url.clone();
        }
        Ok(())
    }

    /// 所有 Bot 的运行变量，Matrix 令牌从还在运行的旧容器读回（升级、按代理重建、名单变了重建都用）。
    /// 没有声明 `bots` 的插件只有一个 Bot，它的身份由调用方放进插件级变量。
    pub(in crate::plugin) async fn bot_runtimes_read_back(
        &self,
        package: &PluginPackage,
        base: &super::render::InstallContext,
        langs: &Languages,
    ) -> Result<BTreeMap<String, BotVars>, InstallError> {
        let manifest = &package.manifest;
        if !is_bot(manifest) {
            return Ok(BTreeMap::new());
        }
        let id = &manifest.id;
        let records = self.store.bots().load(id)?;
        if records.bots.is_empty() {
            return Err(super::egress::cannot_read(
                langs,
                crate::l10n::text(langs, "plugin-settings-egress-bot-record-missing", &[]),
            ));
        }
        let single = manifest.bots.is_none();
        let mut bots = BTreeMap::new();
        for (bot, entry) in &records.bots {
            let mut runtime = self.stored_bot_runtime(package, bot, entry)?;
            // 用占位令牌渲染一遍，找出哪些服务引用了令牌，再从旧容器读回真值。
            let mut probing = base.clone();
            if single {
                probing.matrix = runtime.matrix.clone();
                probing.mcp_url = runtime.mcp_url.clone();
            }
            probing.bots.insert(bot.clone(), runtime.clone());
            let plan = render(package, &probing, langs)?;
            if let (Some(token), Some(matrix)) = (
                self.read_bot_token(&plan, bot, single, langs).await?,
                runtime.matrix.as_mut(),
            ) {
                matrix.access_token = token;
            }
            bots.insert(bot.clone(), runtime);
        }
        Ok(bots)
    }

    /// 从还在运行的旧容器里读回一个 Bot 的 Matrix 令牌：容器按计划里的 Bot 标签对应；旧容器按没有
    /// `--<Bot>` 后缀的名字再找一次（新版本才开始声明 `bots` 的插件）。没有服务引用令牌时不用读。
    async fn read_bot_token(
        &self,
        plan: &super::render::Deployment,
        bot: &str,
        single: bool,
        langs: &Languages,
    ) -> Result<Option<String>, InstallError> {
        let owners: std::collections::HashMap<&str, Option<&str>> = plan
            .containers
            .iter()
            .map(|c| (c.name.as_str(), c.bot.as_deref()))
            .collect();
        let mut token: Option<String> = None;
        for use_ in super::egress::token_uses(plan, langs)
            .map_err(|reason| super::egress::cannot_read(langs, reason))?
        {
            let owner = owners.get(use_.container.as_str()).copied().flatten();
            if !single && owner != Some(bot) {
                continue;
            }
            let mut name = use_.container.clone();
            let mut info = self.engine.inspect_container(&name).await?;
            let default_suffix = format!("--{bot}");
            if info.is_none()
                && (single || bot == super::bot_services::DEFAULT_BOT)
                && name.ends_with(&default_suffix)
            {
                name.truncate(name.len() - default_suffix.len());
                info = self.engine.inspect_container(&name).await?;
            }
            let info = info.ok_or_else(|| {
                super::egress::cannot_read(
                    langs,
                    crate::l10n::text(
                        langs,
                        "plugin-settings-egress-old-container-gone",
                        &[("container", name.as_str().into())],
                    ),
                )
            })?;
            let env = info.config.and_then(|c| c.env).unwrap_or_default();
            let found = use_.read(&env).ok_or_else(|| {
                super::egress::cannot_read(
                    langs,
                    crate::l10n::text(
                        langs,
                        "plugin-settings-egress-token-not-in-env",
                        &[
                            ("container", name.as_str().into()),
                            ("variable", use_.variable.as_str().into()),
                        ],
                    ),
                )
            })?;
            if token.as_ref().is_some_and(|t| *t != found) {
                return Err(super::egress::cannot_read(
                    langs,
                    crate::l10n::text(langs, "plugin-settings-egress-token-mismatch", &[]),
                ));
            }
            token = Some(found);
        }
        Ok(token)
    }

    /// 存下来的一个 Bot 的变量：Matrix 身份的令牌是占位值（读回后替换），模型凭据和总线地址照记录。
    fn stored_bot_runtime(
        &self,
        package: &PluginPackage,
        bot: &str,
        entry: &BotEntry,
    ) -> Result<BotVars, InstallError> {
        let manifest = &package.manifest;
        let model_key = (manifest.permissions.contains(&Permission::Models)
            && manifest.bots.is_some())
        .then(|| {
            let name = format!("{}/{bot}", manifest.id);
            self.store.model_keys().stored_key(&manifest.id, &name)
        })
        .flatten();
        Ok(BotVars {
            model_key,
            matrix: Some(self.stored_matrix_vars(entry)?),
            mcp_url: Some(mcp_url(self.cfg.gateway_port, &entry.bus_token)),
        })
    }

    /// 部署一个新 Bot 的容器：渲染时只放这一个 Bot 的变量，启动它的服务。
    async fn start_bot_containers(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        bot: &str,
        runtime: BotVars,
    ) -> Result<(), InstallError> {
        let langs = self.owner_languages();
        let lan = self.lan_grant(package, record.lan_webui, &langs).await?;
        let mut ctx = self.rebuild_context(package, record, lan, None).await?;
        ctx.bots.insert(bot.to_owned(), runtime);
        let mut plan = render(package, &ctx, &langs)?;
        plan.containers.retain(|c| c.bot.as_deref() == Some(bot));
        let mut undo = Undo::default();
        let started = async {
            self.stage(&package.manifest.id, InstallStage::Preparing);
            self.prepare_images(&mut plan, Some(&package.manifest.id), &langs)
                .await?;
            self.stage(&package.manifest.id, InstallStage::Preparing);
            self.prepare_dirs(&ctx, &plan.dirs, &mut undo, &langs)?;
            // 镜像自带的 healthcheck 也算进时限：和 replace_bot_containers 一样先并入计划。
            self.rewrite_image_healthchecks(&ctx, &mut plan).await?;
            let deadline =
                health_gate::deadline(&plan, ctx.install_limit, tokio::time::Instant::now());
            self.stage(&package.manifest.id, InstallStage::Starting);
            Box::pin(self.start_containers(
                Some(&package.manifest),
                &plan,
                Some(&package.manifest.id),
                &mut undo,
                deadline,
                &langs,
            ))
            .await
        }
        .await;
        if let Err(e) = started {
            let failures = self.rollback(undo, &langs).await;
            let error = if failures.is_empty() {
                e
            } else {
                e.with_note(
                    &langs,
                    &format!("回滚未完成，可能有残留：{}", failures.join("；")),
                )
            };
            return Err(error);
        }
        Ok(())
    }

    /// 只重建一个 Bot 的容器：删掉它的，按计划换新的（镜像先拉好，拉不到时旧容器还在跑）。
    #[allow(dead_code)]
    async fn replace_bot_containers(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        bot: &str,
        runtime: BotVars,
    ) -> Result<(), InstallError> {
        let langs = self.owner_languages();
        let lan = self.lan_grant(package, record.lan_webui, &langs).await?;
        let mut ctx = self.rebuild_context(package, record, lan, None).await?;
        ctx.bots.insert(bot.to_owned(), runtime);
        let mut plan = render(package, &ctx, &langs)?;
        plan.containers.retain(|c| c.bot.as_deref() == Some(bot));
        let id = &package.manifest.id;
        let old = self.bot_containers(id, bot).await?;
        self.stage(id, InstallStage::Preparing);
        self.prepare_images(&mut plan, Some(id), &langs).await?;
        self.stage(id, InstallStage::Preparing);
        for name in &old {
            self.engine.remove_container(name).await?;
        }
        self.rewrite_image_healthchecks(&ctx, &mut plan).await?;
        let deadline = health_gate::deadline(&plan, ctx.install_limit, tokio::time::Instant::now());
        self.stage(id, InstallStage::Starting);
        Box::pin(self.start_containers(
            Some(&package.manifest),
            &plan,
            Some(id),
            &mut Undo::default(),
            deadline,
            &langs,
        ))
        .await?;
        Ok(())
    }

    /// 这个 Bot 的容器名：按标签找（`wonton.bot`），实例不是这套核心的不算。
    async fn bot_containers(
        &self,
        plugin: &PluginId,
        bot: &str,
    ) -> Result<Vec<String>, InstallError> {
        let instance = &self.cfg.instance;
        let found = self
            .engine
            .labeled(super::names::LABEL_PLUGIN, plugin.as_str(), |labels| {
                instance.owns(labels)
                    && labels.get(super::render::LABEL_BOT).map(String::as_str) == Some(bot)
            })
            .await?;
        Ok(found.containers)
    }

    /// 安装时代默认 Bot 登录（归属人是主人），失败时回滚吊销设备。没有 Bot 的插件返回 `None`。
    pub(in crate::plugin) async fn provision_default_bot(
        &self,
        package: &PluginPackage,
        undo: &mut Undo,
    ) -> Result<Option<ProvisionedBot>, InstallError> {
        let manifest = &package.manifest;
        if !is_bot(manifest) {
            return Ok(None);
        }
        let keeper = BotKeeper::User {
            username: self.cfg.owner.clone(),
        };
        let bot = self
            .provision_bot(
                &manifest.id,
                manifest,
                super::bot_services::DEFAULT_BOT,
                keeper,
                package,
            )
            .await?;
        undo.bot = Some((
            manifest.id.clone(),
            super::bot_services::DEFAULT_BOT.to_owned(),
            bot.clone(),
        ));
        Ok(Some(bot))
    }
}
