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
}
