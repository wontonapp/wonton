//! Bot-scoped OIDC registrations and rendering credentials.

use wonton_protocol::{BotRef, bot_web_label};

use super::bot_web_registry::{client_id, has_bot_web};
use super::manager::{InstallError, PluginManager};
use super::manifest::{Manifest, PluginId};
use super::render::InstallContext;
use super::store::StoreError;

impl PluginManager {
    pub fn bot_web_oidc_identity(&self, client: &str) -> Result<Option<BotRef>, StoreError> {
        if let Some(entry) = self
            .web_registrations()?
            .values()
            .find(|entry| client_id(&entry.bot) == client)
        {
            return Ok(Some(entry.bot.clone()));
        }
        let Ok(id) = PluginId::parse(client) else {
            return Ok(None);
        };
        self.bot_web_identity(&id.slug())
    }

    /// Caller holds bot_allocation when creating a reservation or its secret.
    pub(super) fn fill_bot_web_vars(
        &self,
        manifest: &Manifest,
        ctx: &mut InstallContext,
    ) -> Result<(), InstallError> {
        if !has_bot_web(manifest) {
            return Ok(());
        }
        let registrations = self.web_registrations()?;
        for (name, runtime) in &mut ctx.bots {
            let bot = BotRef {
                plugin_id: manifest.id.to_string(),
                bot: name.clone(),
            };
            let label = bot_web_label(&bot);
            runtime.app_url = Some(self.cfg.public.origin(&self.cfg.public.plugin_host(&label)));
            if let Some(base) = &ctx.oidc {
                let registration = registrations.get(&label).ok_or_else(|| {
                    InstallError::Storage(format!(
                        "Missing web registration for {}/{}",
                        manifest.id, name
                    ))
                })?;
                let mut oidc = base.clone();
                oidc.client_id = client_id(&bot);
                oidc.client_secret = if name == "default" {
                    base.client_secret.clone()
                } else {
                    registration.secret.clone().ok_or_else(|| {
                        InstallError::Storage(format!(
                            "Missing OIDC secret for {}/{}",
                            manifest.id, name
                        ))
                    })?
                };
                runtime.oidc = Some(oidc);
            }
        }
        Ok(())
    }

    pub(super) async fn sync_bot_web_clients(
        &self,
        manifest: &Manifest,
        ctx: &InstallContext,
    ) -> Result<(), InstallError> {
        if !has_bot_web(manifest) || ctx.oidc.is_none() {
            return Ok(());
        }
        for runtime in ctx.bots.values() {
            let oidc = runtime
                .oidc
                .as_ref()
                .ok_or_else(|| InstallError::Storage("Missing Bot OIDC runtime".to_owned()))?;
            let base = runtime
                .app_url
                .as_deref()
                .ok_or_else(|| InstallError::Storage("Missing Bot web origin".to_owned()))?;
            let mut spec = super::lifecycle::client_spec_at(manifest, base, &oidc.client_secret);
            spec.id = oidc.client_id.clone();
            self.hydra(&self.owner_languages())?
                .upsert_client(&spec)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn delete_bot_web_clients(
        &self,
        id: &PluginId,
        name: Option<&str>,
    ) -> Result<(), InstallError> {
        for entry in self.web_registrations()?.values().filter(|entry| {
            entry.bot.plugin_id == id.as_str() && name.is_none_or(|name| entry.bot.bot == name)
        }) {
            if let Some(hydra) = &self.oidc {
                hydra.delete_client(&client_id(&entry.bot)).await?;
            }
        }
        if name.is_none() {
            self.clear_bot_web_routes(id);
        }
        Ok(())
    }

    pub(super) async fn drop_unused_bot_web(
        &self,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        let _allocating = self.bot_allocation.lock().await;
        for entry in self
            .web_registrations()?
            .values()
            .filter(|entry| entry.bot.plugin_id == manifest.id.as_str())
        {
            if entry.bot.bot == "default" && super::upstreams::uses_sso(manifest) {
                continue;
            }
            if let Some(hydra) = &self.oidc {
                hydra.delete_client(&client_id(&entry.bot)).await?;
            }
        }
        if !has_bot_web(manifest) {
            self.release_bot_web(&manifest.id, None)?;
        }
        Ok(())
    }
}
