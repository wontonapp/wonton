//! Exact Bot web targets. Availability is runtime fact, never an authorization decision.

use wonton_protocol::{BotRef, BotWebFailure, BotWebPage, bot_web_label};

use super::bot_web_registry::has_bot_web;
use super::health::{self, Checked};
use super::manager::{InstallError, PluginManager, PluginStatus, Route};
use super::manifest::{Manifest, PluginId};
use super::store::StoreError;
use super::upstreams::Endpoint;

#[derive(Debug, thiserror::Error)]
pub enum BotWebError {
    #[error("Bot webpage unavailable: {0:?}")]
    Unavailable(BotWebFailure),
    #[error(transparent)]
    Storage(#[from] StoreError),
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct BotRoute {
    pub bot: BotRef,
    endpoint: Endpoint,
    ready: bool,
}

impl PluginManager {
    pub fn bot_web_identity(&self, label: &str) -> Result<Option<BotRef>, StoreError> {
        Ok(self
            .web_registrations()?
            .get(label)
            .map(|entry| entry.bot.clone()))
    }

    pub fn bot_web_target(&self, bot: &BotRef) -> Result<(Route, BotWebPage), BotWebError> {
        let unavailable = BotWebError::Unavailable;
        let id =
            PluginId::parse(&bot.plugin_id).map_err(|_| unavailable(BotWebFailure::NotFound))?;
        let record = self
            .store
            .load(&id)?
            .ok_or(unavailable(BotWebFailure::NotFound))?;
        if !self.store.bots().load(&id)?.bots.contains_key(&bot.bot) {
            return Err(unavailable(BotWebFailure::NotFound));
        }
        let package = super::upgrade::installed_package(&record, &self.owner_languages()).map_err(
            |error| StoreError::Corrupt {
                path: self.store.bot_web_registry(),
                message: error.to_string(),
            },
        )?;
        if !has_bot_web(&package.manifest) {
            return Err(unavailable(BotWebFailure::NoWebui));
        }
        let web = package.manifest.webui.as_ref().expect("Bot web checked");
        let mut route = self
            .route(&id.slug(), "/")
            .ok_or(unavailable(BotWebFailure::NotReady))?;
        if !matches!(
            route.status,
            PluginStatus::Running | PluginStatus::Degraded { .. }
        ) {
            return Err(unavailable(BotWebFailure::NotReady));
        }
        let label = bot_web_label(bot);
        let routes = self.bot_web_routes.read().expect("Bot web routes poisoned");
        let target = routes
            .get(&label)
            .filter(|target| target.bot == *bot && target.ready)
            .ok_or(unavailable(BotWebFailure::NotReady))?;
        route.upstream = Some(target.endpoint.clone());
        route.web_serving = true;
        let page = BotWebPage {
            bot: bot.clone(),
            url: format!(
                "{}{}",
                self.cfg.public.origin(&self.cfg.public.plugin_host(&label)),
                web.path
            ),
            cross_origin_isolated: web.cross_origin_isolated,
        };
        Ok((route, page))
    }

    pub(super) async fn refresh_bot_web_routes(
        &self,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        if !has_bot_web(manifest) {
            self.clear_bot_web_routes(&manifest.id);
            return Ok(());
        }
        let web = manifest.webui.as_ref().expect("Bot web checked");
        let slug = manifest.id.slug();
        let network = self.cfg.instance.network(&slug);
        let mut routes = std::collections::BTreeMap::new();
        for name in self.store.bots().load(&manifest.id)?.bots.keys() {
            let bot = BotRef {
                plugin_id: manifest.id.to_string(),
                bot: name.clone(),
            };
            let container = self.cfg.instance.bot_container(&slug, &web.service, name);
            let checked = health::check_now(
                &self.engine,
                &container,
                &web.service,
                false,
                &self.owner_languages(),
            )
            .await?;
            let host = super::names::service_host(
                &slug,
                &super::bot_services::bot_alias(&web.service, name),
            );
            if let Some(address) = self
                .service_address(&container, &host, web.port, &network)
                .await?
            {
                routes.insert(
                    bot_web_label(&bot),
                    BotRoute {
                        bot,
                        endpoint: Endpoint::Tcp(address),
                        ready: matches!(checked, Checked::Healthy | Checked::Running),
                    },
                );
            }
        }
        let mut stored = self
            .bot_web_routes
            .write()
            .expect("Bot web routes poisoned");
        let changed = stored
            .iter()
            .filter(|(_, route)| route.bot.plugin_id == manifest.id.as_str())
            .ne(routes.iter());
        stored.retain(|_, route| route.bot.plugin_id != manifest.id.as_str());
        stored.extend(routes);
        drop(stored);
        if changed {
            self.changed();
        }
        Ok(())
    }

    pub(super) async fn wait_bot_web(
        &self,
        manifest: &Manifest,
        name: &str,
        deadline: tokio::time::Instant,
    ) -> Result<(), InstallError> {
        if !has_bot_web(manifest) {
            return Ok(());
        }
        let web = manifest.webui.as_ref().expect("Bot web checked");
        let slug = manifest.id.slug();
        let container = self.cfg.instance.bot_container(&slug, &web.service, name);
        let network = self.cfg.instance.network(&slug);
        let host =
            super::names::service_host(&slug, &super::bot_services::bot_alias(&web.service, name));
        let address = self
            .service_address(&container, &host, web.port, &network)
            .await?
            .ok_or_else(|| {
                InstallError::Unhealthy(format!("Bot {name} webpage has no upstream"))
            })?;
        health::wait_webui(
            &self.http,
            &Endpoint::Tcp(address),
            &web.path,
            deadline,
            &self.owner_languages(),
        )
        .await?;
        Ok(())
    }

    pub(super) fn clear_bot_web_routes(&self, id: &PluginId) {
        self.bot_web_routes
            .write()
            .expect("Bot web routes poisoned")
            .retain(|_, route| route.bot.plugin_id != id.as_str());
    }
}

#[cfg(test)]
impl PluginManager {
    pub(crate) fn bot_web_route_for_test(&self, bot: BotRef, endpoint: Endpoint, ready: bool) {
        self.bot_web_routes
            .write()
            .expect("Bot web routes poisoned")
            .insert(
                bot_web_label(&bot),
                BotRoute {
                    bot,
                    endpoint,
                    ready,
                },
            );
    }
}
