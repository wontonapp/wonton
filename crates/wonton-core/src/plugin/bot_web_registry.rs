//! Collision-checked web identities, retained with Bot records across uninstall/reinstall.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use wonton_protocol::{BotRef, bot_web_label, bot_web_registration};

use super::manager::{InstallError, PluginManager};
use super::manifest::{Manifest, PluginId};
use super::store::{StoreError, read_json, write_json};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Registration {
    pub bot: BotRef,
    pub secret: Option<String>,
}

pub(super) type Registrations = BTreeMap<String, Registration>;

pub(super) fn has_bot_web(manifest: &Manifest) -> bool {
    manifest.webui.as_ref().is_some_and(|web| {
        super::bot_services::bot_services(manifest).contains(web.service.as_str())
    })
}

pub(super) fn client_id(bot: &BotRef) -> String {
    if bot.bot == super::bot_services::DEFAULT_BOT {
        bot.plugin_id.clone()
    } else {
        bot_web_label(bot)
    }
}

impl PluginManager {
    pub(super) async fn prune_orphaned_bot_web(&self) -> Result<(), InstallError> {
        let _allocating = self.bot_allocation.lock().await;
        let mut registrations = self.web_registrations()?;
        let mut orphaned = Vec::new();
        for (label, entry) in &registrations {
            let id = PluginId::parse(&entry.bot.plugin_id)
                .map_err(|error| InstallError::Storage(error.to_string()))?;
            let recorded = self
                .store
                .bots()
                .load(&id)?
                .bots
                .contains_key(&entry.bot.bot);
            let installed = self.store.load(&id)?.is_some();
            let installed_default = entry.bot.bot == "default" && installed;
            if !installed || (!recorded && !installed_default) {
                if let Some(hydra) = &self.oidc {
                    hydra.delete_client(&client_id(&entry.bot)).await?;
                }
                if !recorded && !installed_default {
                    orphaned.push(label.clone());
                }
            }
        }
        for label in orphaned {
            registrations.remove(&label);
        }
        self.save_web_registrations(&registrations)?;
        Ok(())
    }

    pub(super) fn runtime_core_aliases(
        &self,
        manifest: &Manifest,
    ) -> Result<Vec<String>, StoreError> {
        let mut aliases = super::upstreams::core_aliases(&self.cfg.public, manifest);
        if has_bot_web(manifest) {
            for entry in self
                .web_registrations()?
                .values()
                .filter(|entry| entry.bot.plugin_id == manifest.id.as_str())
            {
                let host = self.cfg.public.plugin_host(&bot_web_label(&entry.bot));
                if !aliases.contains(&host) {
                    aliases.push(host);
                }
            }
        }
        Ok(aliases)
    }

    pub(super) fn web_registrations(&self) -> Result<Registrations, StoreError> {
        Ok(read_json(&self.store.bot_web_registry())?.unwrap_or_default())
    }

    pub(super) fn save_web_registrations(
        &self,
        registrations: &Registrations,
    ) -> Result<(), StoreError> {
        let path = self.store.bot_web_registry();
        super::store::create_private_dir(path.parent().expect("registry parent"))?;
        write_json(&path, registrations)
    }

    pub(super) fn reserve_manifest_bot_web(&self, manifest: &Manifest) -> Result<(), InstallError> {
        if !has_bot_web(manifest) {
            return Ok(());
        }
        self.reserve_bot_web(manifest, "default")?;
        for name in self.store.bots().load(&manifest.id)?.bots.keys() {
            self.reserve_bot_web(manifest, name)?;
        }
        Ok(())
    }

    pub(super) async fn ensure_bot_web_registration(
        &self,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        let _allocating = self.bot_allocation.lock().await;
        self.reserve_manifest_bot_web(manifest)
    }

    /// Caller holds bot_allocation; persist the reservation before any external provisioning.
    pub(super) fn reserve_bot_web(
        &self,
        manifest: &Manifest,
        name: &str,
    ) -> Result<(), InstallError> {
        if !has_bot_web(manifest) {
            return Ok(());
        }
        let bot = BotRef {
            plugin_id: manifest.id.to_string(),
            bot: name.to_owned(),
        };
        let mut registrations = self.web_registrations()?;
        // Older installed records reconstruct the namespace before a new allocation can use it.
        for file in self.store.load_all()? {
            let record = file.record?;
            let package = super::upgrade::installed_package(&record, &self.owner_languages())?;
            if !has_bot_web(&package.manifest) {
                continue;
            }
            register(
                &mut registrations,
                BotRef {
                    plugin_id: record.id.to_string(),
                    bot: "default".to_owned(),
                },
                None,
            )?;
            for name in self.store.bots().load(&record.id)?.bots.keys() {
                let identity = BotRef {
                    plugin_id: record.id.to_string(),
                    bot: name.clone(),
                };
                register(&mut registrations, identity, None)?;
            }
        }
        let secret = (super::upstreams::uses_sso(manifest)
            && name != "default"
            && registrations
                .get(&bot_web_label(&bot))
                .is_none_or(|entry| entry.secret.is_none()))
        .then(crate::secret::random_token);
        register(&mut registrations, bot, secret)?;
        self.save_web_registrations(&registrations)?;
        Ok(())
    }

    pub(super) fn release_bot_web(
        &self,
        id: &PluginId,
        name: Option<&str>,
    ) -> Result<(), StoreError> {
        let mut registrations = self.web_registrations()?;
        registrations.retain(|_, entry| {
            entry.bot.plugin_id != id.as_str() || name.is_some_and(|name| entry.bot.bot != name)
        });
        self.save_web_registrations(&registrations)?;
        self.bot_web_routes
            .write()
            .expect("Bot web routes poisoned")
            .retain(|_, route| {
                route.bot.plugin_id != id.as_str() || name.is_some_and(|name| route.bot.bot != name)
            });
        Ok(())
    }
}

fn register(
    registrations: &mut Registrations,
    bot: BotRef,
    secret: Option<String>,
) -> Result<(), InstallError> {
    let label = bot_web_label(&bot);
    bot_web_registration(&bot, registrations.get(&label).map(|entry| &entry.bot))
        .map_err(|error| InstallError::Conflict(error.to_string()))?;
    let registration = registrations
        .entry(label)
        .or_insert(Registration { bot, secret: None });
    if registration.secret.is_none() {
        registration.secret = secret;
    }
    Ok(())
}
