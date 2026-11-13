//! Recoverable member Bot creation/removal shares the ordinary Bot lifecycle.

use wonton_protocol::{BotKeeper, BotRef};

use super::bot_services::{BotVars, DEFAULT_BOT};
use super::bots::{BotEntry, check_bot_name};
use super::bots_lifecycle::BotCreated;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::{PluginId, PluginKind};
use super::package::PluginPackage;
use super::render::{Deployment, render};
use super::store::InstalledRecord;
use super::upgrade::installed_package;
use crate::matrix::ProvisionedBot;

impl PluginManager {
    pub async fn ensure_member_bot(
        &self,
        plugin: &PluginId,
        bot: &str,
        username: &str,
    ) -> Result<BotRef, InstallError> {
        self.ensure_bot(
            plugin,
            bot,
            BotKeeper::User {
                username: username.to_owned(),
            },
            true,
        )
        .await?;
        Ok(BotRef {
            plugin_id: plugin.to_string(),
            bot: bot.to_owned(),
        })
    }

    pub(super) async fn ensure_bot(
        &self,
        plugin: &PluginId,
        bot: &str,
        keeper: BotKeeper,
        existing_allowed: bool,
    ) -> Result<BotCreated, InstallError> {
        check_bot_name(bot).map_err(InstallError::Invalid)?;
        if bot == DEFAULT_BOT {
            return Err(InstallError::Invalid(format!(
                "{bot} 是安装时默认 Bot 的名字，换一个"
            )));
        }
        let _allocating = self.bot_allocation.lock().await;
        let record = self
            .store
            .load(plugin)?
            .ok_or_else(|| InstallError::NotInstalled(plugin.to_string()))?;
        let langs = self.owner_languages();
        let package = installed_package(&record, &langs)?;
        if package.manifest.bots.is_none() || package.manifest.kind != PluginKind::Agent {
            return Err(InstallError::Invalid(format!(
                "插件 {plugin} 没有声明 bots，不支持多个 Bot"
            )));
        }
        let existing = self.store.bots().entry(plugin, bot)?;
        if self.store.bots().retired(plugin, bot)?.is_some() {
            return Err(InstallError::Conflict(format!(
                "插件 {plugin} 已经有 Bot {bot}"
            )));
        }
        let pending = self.store.bots().pending(plugin)?.contains_key(bot);
        let recovering = existing_allowed && (existing.is_some() || pending);
        let status = self.status_of(plugin);
        if let Some(status) = &status
            && let Some(error) = super::manager::busy(plugin, status, &langs)
        {
            return Err(error);
        }
        if status != Some(PluginStatus::Running)
            && !(recovering && matches!(status, Some(PluginStatus::Degraded { .. })))
        {
            return Err(InstallError::Invalid(format!(
                "插件 {plugin} 没在运行，先启动再建 Bot"
            )));
        }
        if let Some(entry) = &existing {
            if entry.keeper != keeper {
                return Err(self.home_denied());
            }
            if let Some(provisioned) = self.running_bot(&package, &record, bot, entry).await? {
                if !existing_allowed {
                    return Err(InstallError::Conflict(format!(
                        "插件 {plugin} 已经有 Bot {bot}"
                    )));
                }
                self.hand_over_home_room(&self.keeper_user(&keeper), &provisioned)
                    .await?;
                return Ok(created(bot, provisioned));
            }
            for name in self.bot_containers(plugin, bot).await? {
                self.engine.remove_container(&name).await?;
            }
            self.revoke_member_device(plugin, bot).await?;
        }
        // Resolve eligibility before allocating any external credentials.
        if let Some(write) = super::private_homes::home_access(&package.manifest.permissions) {
            self.keeper_home(&keeper, write)?;
        }
        let provisioned = self
            .provision_bot_locked(plugin, &package.manifest, bot, keeper.clone(), &package)
            .await?;
        let result = self
            .finish_bot(&package, &record, bot, &keeper, &provisioned)
            .await;
        if let Err(error) = result {
            // Retain the account and room for retry, but never leave a usable orphan device.
            for name in self.bot_containers(plugin, bot).await? {
                self.engine.remove_container(&name).await?;
            }
            self.revoke_bot_device(plugin, &provisioned)
                .await
                .map_err(InstallError::Storage)?;
            self.revoke_bot_credential(plugin, bot, &langs).await?;
            return Err(error);
        }
        self.changed();
        Ok(created(bot, provisioned))
    }

    async fn revoke_member_device(&self, plugin: &PluginId, bot: &str) -> Result<(), InstallError> {
        let mut records = self.store.bots().load(plugin)?;
        if let Some(entry) = records.bots.get_mut(bot) {
            if let Some(device) = &entry.device_id {
                self.matrix()?
                    .delete_device(&entry.account.user_id, device)
                    .await?;
            }
            entry.device_id = None;
            self.store.bots().save(plugin, &records)?;
        }
        Ok(())
    }

    async fn finish_bot(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        bot: &str,
        keeper: &BotKeeper,
        provisioned: &ProvisionedBot,
    ) -> Result<(), InstallError> {
        let model_key = self
            .issue_bot_credential(package, bot, &self.owner_languages())
            .await?;
        let entry = self
            .store
            .bots()
            .entry(&package.manifest.id, bot)?
            .ok_or_else(|| InstallError::NotFound(bot.to_owned()))?;
        let runtime = BotVars {
            model_key,
            matrix: Some(self.matrix_vars(provisioned)),
            mcp_url: Some(format!(
                "http://{}:{}/mcp/{}",
                crate::config::PLUGIN_MCP_HOST,
                self.cfg.gateway_port,
                entry.bus_token
            )),
        };
        self.start_bot_containers(package, record, bot, runtime)
            .await?;
        self.hand_over_home_room(&self.keeper_user(keeper), provisioned)
            .await?;
        Ok(())
    }

    async fn running_bot(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        bot: &str,
        entry: &BotEntry,
    ) -> Result<Option<ProvisionedBot>, InstallError> {
        let Some(device) = &entry.device_id else {
            return Ok(None);
        };
        if !self
            .matrix()?
            .devices(&entry.account.user_id)
            .await?
            .contains(device)
        {
            return Ok(None);
        }
        let runtime = self.stored_bot_runtime(package, bot, entry)?;
        let plan = self.member_bot_plan(package, record, bot, runtime).await?;
        if plan.containers.is_empty() {
            return Err(InstallError::NotFound(bot.to_owned()));
        }
        for container in &plan.containers {
            let Some(info) = self.engine.inspect_container(&container.name).await? else {
                return Ok(None);
            };
            let Some(state) = info.state else {
                return Ok(None);
            };
            if container.one_shot {
                if state.exit_code != Some(0) || state.running == Some(true) {
                    return Ok(None);
                }
            } else if state.running != Some(true) {
                return Ok(None);
            }
            if state
                .health
                .and_then(|health| health.status)
                .is_some_and(|status| status != bollard::models::HealthStatusEnum::HEALTHY)
            {
                return Ok(None);
            }
        }
        let Some(token) = self
            .read_bot_token(&plan, bot, false, &self.owner_languages())
            .await?
        else {
            return Err(super::egress::cannot_read(
                &self.owner_languages(),
                crate::l10n::text(
                    &self.owner_languages(),
                    "plugin-settings-egress-token-not-used",
                    &[],
                ),
            ));
        };
        Ok(Some(ProvisionedBot {
            account: entry.account.clone(),
            device_id: device.clone(),
            access_token: token,
            owner: self.matrix()?.user_id(&self.keeper_user(&entry.keeper)),
        }))
    }

    async fn member_bot_plan(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        bot: &str,
        runtime: BotVars,
    ) -> Result<Deployment, InstallError> {
        let langs = self.owner_languages();
        let lan = self.lan_grant(package, record.lan_webui, &langs).await?;
        let mut ctx = self.rebuild_context(package, record, lan, None).await?;
        ctx.bots.clear();
        ctx.bots.insert(bot.to_owned(), runtime);
        let mut plan = render(package, &ctx, &langs)?;
        plan.containers
            .retain(|container| container.bot.as_deref() == Some(bot));
        Ok(plan)
    }

    pub async fn remove_member_bot(
        &self,
        reference: &BotRef,
        username: &str,
        keep_data: bool,
    ) -> Result<(), InstallError> {
        let id = PluginId::parse(&reference.plugin_id)
            .map_err(|e| InstallError::Invalid(e.to_string()))?;
        let _allocating = self.bot_allocation.lock().await;
        self.remove_bot_locked(&id, &reference.bot, keep_data, Some(username))
            .await?;
        Ok(())
    }

    pub async fn sync_member_bot_home(&self, reference: &BotRef) -> Result<(), InstallError> {
        let id = PluginId::parse(&reference.plugin_id)
            .map_err(|e| InstallError::Invalid(e.to_string()))?;
        let _allocating = self.bot_allocation.lock().await;
        let result = self.sync_home_locked(&id, &reference.bot).await;
        if result.is_err() {
            // A failed downgrade cannot leave the previously writable mount available.
            for name in self.bot_containers(&id, &reference.bot).await? {
                self.engine.remove_container(&name).await?;
            }
        }
        result
    }

    async fn sync_home_locked(&self, id: &PluginId, bot: &str) -> Result<(), InstallError> {
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        let package = installed_package(&record, &self.owner_languages())?;
        let entry = self
            .store
            .bots()
            .entry(id, bot)?
            .ok_or_else(|| InstallError::NotFound(bot.to_owned()))?;
        let mut runtime = self.stored_bot_runtime(&package, bot, &entry)?;
        let plan = self
            .member_bot_plan(&package, &record, bot, runtime.clone())
            .await?;
        let token = self
            .read_bot_token(&plan, bot, false, &self.owner_languages())
            .await?;
        if let (Some(token), Some(matrix)) = (token, runtime.matrix.as_mut()) {
            matrix.access_token = token;
        }
        let plan = self
            .member_bot_plan(&package, &record, bot, runtime.clone())
            .await?;
        let mut changed = false;
        for container in &plan.containers {
            let info = self.engine.inspect_container(&container.name).await?;
            let actual = info
                .as_ref()
                .and_then(|info| info.host_config.as_ref())
                .and_then(|host| host.mounts.as_ref());
            let expected = container
                .body
                .host_config
                .as_ref()
                .and_then(|host| host.mounts.as_ref());
            changed |= home_mounts(actual) != home_mounts(expected);
            let digest = info
                .and_then(|info| info.config)
                .and_then(|config| config.labels)
                .and_then(|labels| labels.get(super::render_drift::LABEL_RENDER).cloned());
            changed |= digest.as_deref() != Some(super::render_drift::digest(container).as_str());
        }
        if changed {
            self.replace_bot_containers(&package, &record, bot, runtime)
                .await?;
        }
        Ok(())
    }
}

fn created(bot: &str, provisioned: ProvisionedBot) -> BotCreated {
    BotCreated {
        bot: bot.to_owned(),
        user_id: provisioned.account.user_id,
        home_room: provisioned.account.home_room,
    }
}

fn home_mounts(mounts: Option<&Vec<bollard::models::Mount>>) -> Option<(&str, bool)> {
    mounts
        .into_iter()
        .flatten()
        .find(|mount| mount.target.as_deref() == Some(super::private_homes::HOME_TARGET))
        .map(|mount| {
            (
                mount.source.as_deref().unwrap_or(""),
                mount.read_only.unwrap_or(false),
            )
        })
}
