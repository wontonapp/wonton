//! Installation preparation and rollback of Bot-scoped web resources.

use super::lifecycle::Undo;
use super::manager::{InstallError, PluginManager};
use super::package::PluginPackage;
use super::render::InstallContext;

impl PluginManager {
    pub(super) async fn prepare_install_bots(
        &self,
        package: &PluginPackage,
        ctx: &mut InstallContext,
        undo: &mut Undo,
    ) -> Result<(), InstallError> {
        let manifest = &package.manifest;
        let default = self.provision_default_bot(package, undo).await?;
        if let Some(models) = self
            .provision_models(package, &self.owner_languages())
            .await?
        {
            ctx.models = Some(models);
            undo.models = Some(manifest.id.clone());
        }
        if let Some(bot) = &default {
            self.fill_default_bot_vars(manifest, bot, ctx)?;
        }
        if manifest.bots.is_some() {
            for (name, entry) in self.store.bots().load(&manifest.id)?.bots {
                if name == "default" {
                    continue;
                }
                let bot = self
                    .provision_bot_locked(&manifest.id, manifest, &name, entry.keeper, package)
                    .await?;
                undo.extra_bots
                    .push((manifest.id.clone(), name.clone(), bot.clone()));
                let mut runtime = self.stored_bot_runtime(
                    package,
                    &name,
                    &self.store.bots().load(&manifest.id)?.bots[&name],
                )?;
                runtime.matrix = Some(self.matrix_vars(&bot));
                ctx.bots.insert(name, runtime);
            }
        }
        self.fill_bot_web_vars(manifest, ctx)
    }

    pub(super) async fn refresh_bot_core_aliases(
        &self,
        manifest: &super::manifest::Manifest,
    ) -> Result<(), InstallError> {
        let network = self.cfg.instance.network(&manifest.id.slug());
        if let Some(core) = &self.cfg.core_container {
            self.attach_core(&network, core, manifest).await?;
        }
        Ok(())
    }

    pub(super) async fn create_bot_runtime(
        &self,
        package: &PluginPackage,
        record: &super::store::InstalledRecord,
        bot: &str,
        keeper: wonton_protocol::BotKeeper,
    ) -> Result<super::bots_lifecycle::BotCreated, InstallError> {
        let manifest = &package.manifest;
        let id = &manifest.id;
        let provisioned = self
            .provision_bot_locked(id, manifest, bot, keeper.clone(), package)
            .await?;
        let result = async {
            let model_key = self
                .issue_bot_credential(package, bot, &self.owner_languages())
                .await?;
            let entries = self.store.bots().load(id)?;
            let entry = entries
                .bots
                .get(bot)
                .ok_or_else(|| InstallError::Storage("Missing provisioned Bot".to_owned()))?;
            let runtime = super::bot_services::BotVars {
                model_key,
                matrix: Some(self.matrix_vars(&provisioned)),
                mcp_url: Some(format!(
                    "http://{}:{}/mcp/{}",
                    crate::config::PLUGIN_MCP_HOST,
                    self.cfg.gateway_port,
                    entry.bus_token
                )),
                ..Default::default()
            };
            self.refresh_bot_core_aliases(manifest).await?;
            self.start_bot_containers(package, record, bot, runtime)
                .await?;
            self.hand_over_home_room(&self.keeper_user(&keeper), &provisioned)
                .await?;
            Ok(super::bots_lifecycle::BotCreated {
                bot: bot.to_owned(),
                user_id: provisioned.account.user_id.clone(),
                home_room: provisioned.account.home_room.clone(),
            })
        }
        .await;
        if result.is_err() {
            for container in self.bot_containers(id, bot).await? {
                self.engine.remove_container(&container).await?;
            }
            self.revoke_bot_credential(id, bot, &self.owner_languages())
                .await?;
            self.revoke_bot_device(id, &provisioned)
                .await
                .map_err(InstallError::Storage)?;
        }
        result
    }
}
