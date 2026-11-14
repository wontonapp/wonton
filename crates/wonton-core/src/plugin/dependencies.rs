//! 直接能力依赖的连接、凭据与提供方选择；模型网关和出口代理仍走各自的专用链路。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use wonton_protocol::{
    BotRef, CapabilityProviderRef, CapabilityProviderSettings, CapabilitySetting,
    ConsumerCapabilitySetting,
};

use super::capabilities;
use super::languages::Languages;
use super::listed::Listed;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::{Manifest, PluginId, PluginKind};
use super::provides::ProviderService;
use super::upstreams::Endpoint;

mod binding;
mod bot_cleanup;
mod bot_connections;
mod credentials;
mod endpoint;
mod install;
mod plan;
mod preflight;
pub(crate) mod preview;
mod routing;
pub(in crate::plugin) use bot_cleanup::BotRemovalRecovery;
pub(crate) use install::DependencyInstall;
mod handoff;
mod lifecycle;
mod provider_token;
mod rollback;
mod selection;
mod store;
mod transaction_recovery;
mod uninstall;
pub(crate) use store::Dependencies;
use store::{Connection, State};
pub(in crate::plugin) struct DependencySnapshot(Vec<Connection>);

#[derive(Debug)]
pub(crate) enum CapabilityAccessError {
    Denied,
    Unavailable(String),
}

pub(crate) struct CapabilityRoute {
    pub provider: PluginId,
    pub endpoint: Endpoint,
    pub bot: Option<BotRef>,
}

#[derive(Clone)]
struct Provider {
    id: PluginId,
    listed: Arc<Listed>,
    status: PluginStatus,
    serving: BTreeSet<String>,
}

pub(crate) fn generic(name: &str) -> bool {
    name != capabilities::MODELS.name && !capabilities::has_no_consumer(name)
}

pub(crate) fn declarations(manifest: &Manifest) -> impl Iterator<Item = &String> {
    manifest
        .requires
        .iter()
        .chain(&manifest.wants)
        .filter(|name| generic(name))
}

fn service<'a>(manifest: &'a Manifest, capability: &str) -> Option<&'a ProviderService> {
    manifest.provides.as_ref()?.capabilities.get(capability)
}

fn high(manifest: &Manifest, capability: &str) -> bool {
    capabilities::lookup(capability).map_or_else(
        || manifest.kind == PluginKind::Agent || manifest.all_permissions().any(|p| p.is_high()),
        |entry| entry.high_privilege,
    )
}

fn reference(provider: &Provider, capability: &str, langs: &Languages) -> CapabilityProviderRef {
    CapabilityProviderRef {
        id: provider.id.to_string(),
        title: langs.pick_owned(&provider.listed.manifest.title),
        high_privilege: high(&provider.listed.manifest, capability),
    }
}

fn chosen<'a>(
    state: &State,
    providers: &'a [Provider],
    consumer: &PluginId,
    capability: &str,
    bot: Option<&BotRef>,
) -> Option<&'a Provider> {
    // ADR-0061 第 9 节：提供方可以 requires/wants 自己也提供的能力，选提供方时排除它自己；
    // 显式选择或全局默认指向自己时，落到别的提供方上。
    let selected = state.selection(
        consumer.as_str(),
        bot.map(|bot| bot.bot.as_str()),
        capability,
    );
    if let Some(binding::Binding::Other(_)) = selected {
        return None;
    }
    if let Some(id) = selected
        .and_then(binding::Binding::provider)
        .or_else(|| state.defaults.get(capability).map(String::as_str))
    {
        let selected = providers
            .iter()
            .find(|p| p.id.as_str() == id && service(&p.listed.manifest, capability).is_some());
        if let Some(provider) = selected.filter(|p| p.id != *consumer) {
            return Some(provider);
        }
        if selected_binding_bot(state, consumer, capability, bot).is_some() {
            return None;
        }
    }
    providers
        .iter()
        .find(|p| p.id != *consumer && service(&p.listed.manifest, capability).is_some())
}

fn selected_binding_bot<'a>(
    state: &'a State,
    consumer: &PluginId,
    capability: &str,
    bot: Option<&BotRef>,
) -> Option<&'a str> {
    state
        .selection(
            consumer.as_str(),
            bot.map(|bot| bot.bot.as_str()),
            capability,
        )
        .and_then(binding::Binding::bot)
}

fn connection<'a>(
    state: &'a State,
    consumer: &PluginId,
    provider: &PluginId,
    capability: &str,
    bot: Option<&BotRef>,
) -> Option<&'a Connection> {
    connection_to(
        state,
        consumer,
        provider,
        capability,
        bot,
        selected_binding_bot(state, consumer, capability, bot),
    )
}

fn connection_to<'a>(
    state: &'a State,
    consumer: &PluginId,
    provider: &PluginId,
    capability: &str,
    bot: Option<&BotRef>,
    provider_bot: Option<&str>,
) -> Option<&'a Connection> {
    state.connections.iter().find(|c| {
        c.consumer == consumer.as_str()
            && c.provider == provider.as_str()
            && c.capability == capability
            && c.consumer_bot.as_deref() == bot.map(|bot| bot.bot.as_str())
            && c.provider_bot.as_deref() == provider_bot
    })
}

impl PluginManager {
    fn capability_providers(&self) -> Vec<Provider> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let mut providers: Vec<_> = plugins
            .iter()
            .filter_map(|(id, entry)| {
                let listed = entry.listed.as_ref()?;
                listed
                    .manifest
                    .provides
                    .as_ref()
                    .filter(|p| !p.capabilities.is_empty())?;
                Some(Provider {
                    id: id.clone(),
                    listed: Arc::clone(listed),
                    status: entry.status.clone(),
                    serving: entry.serving.clone(),
                })
            })
            .collect();
        providers.sort_by(|a, b| {
            a.listed
                .installed_at
                .cmp(&b.listed.installed_at)
                .then(a.id.cmp(&b.id))
        });
        if let Ok(state) = self.dependencies.load() {
            providers.sort_by_key(|p| {
                state
                    .providers
                    .iter()
                    .position(|id| id == p.id.as_str())
                    .unwrap_or(usize::MAX)
            });
        }
        providers
    }

    fn capability_consumers(&self) -> Vec<Arc<Listed>> {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .values()
            .filter_map(|entry| entry.listed.as_ref())
            .filter(|listed| declarations(&listed.manifest).next().is_some())
            .cloned()
            .collect()
    }

    pub fn capability_settings(
        &self,
        langs: &Languages,
    ) -> Result<CapabilityProviderSettings, InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let mut by_capability: BTreeMap<String, Vec<CapabilityProviderRef>> = BTreeMap::new();
        for p in &providers {
            for capability in p
                .listed
                .manifest
                .provides
                .as_ref()
                .into_iter()
                .flat_map(|p| p.capabilities.keys())
            {
                by_capability
                    .entry(capability.clone())
                    .or_default()
                    .push(reference(p, capability, langs));
            }
        }
        Ok(CapabilityProviderSettings {
            capabilities: by_capability
                .into_iter()
                .map(|(capability, providers)| {
                    let default_provider = state
                        .defaults
                        .get(&capability)
                        .filter(|id| providers.iter().any(|p| p.id == **id))
                        .cloned()
                        .unwrap_or_else(|| providers[0].id.clone());
                    CapabilitySetting {
                        third_party: capabilities::is_third_party(&capability),
                        capability,
                        providers,
                        default_provider,
                    }
                })
                .collect(),
        })
    }

    pub fn bot_capability_settings(
        &self,
        consumer: &PluginId,
        bot: &str,
        langs: &Languages,
    ) -> Result<Vec<ConsumerCapabilitySetting>, InstallError> {
        if self.capability_caller_bot(consumer, bot).is_none() {
            return Err(InstallError::NotFound(bot.to_owned()));
        }
        let record = self
            .store
            .load(consumer)?
            .ok_or_else(|| InstallError::not_installed(langs, consumer))?;
        let package = super::upgrade::installed_package(&record, langs)?;
        self.consumer_capabilities(&package.manifest, langs, Some(bot))
    }

    pub(in crate::plugin) fn consumer_capabilities(
        &self,
        manifest: &Manifest,
        langs: &Languages,
        bot: Option<&str>,
    ) -> Result<Vec<ConsumerCapabilitySetting>, InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let scope: std::borrow::Cow<'_, str> = bot.map_or_else(
            || manifest.id.as_str().into(),
            |bot| binding::consumer_key(manifest.id.as_str(), bot).into(),
        );
        let selected = state.selected.get(scope.as_ref());
        Ok(declarations(manifest)
            .map(|capability| ConsumerCapabilitySetting {
                capability: capability.clone(),
                third_party: capabilities::is_third_party(capability),
                // 选提供方时排除自己（ADR-0061 第 9 节）。
                providers: providers
                    .iter()
                    .filter(|p| {
                        p.id != manifest.id && service(&p.listed.manifest, capability).is_some()
                    })
                    .map(|p| reference(p, capability, langs))
                    .collect(),
                selected: selected
                    .and_then(|entries| entries.get(capability))
                    .and_then(binding::Binding::provider)
                    .map(str::to_owned),
            })
            .collect())
    }

    fn capability_missing(&self, capability: &str, langs: &Languages) -> String {
        crate::l10n::text(
            langs,
            "plugin-capability-missing",
            &[("capability", capability.into())],
        )
    }

    /// 缺提供方的错误；清单自己也提供这个能力时，写明要的是另一个提供方（ADR-0061 第 9 节）。
    pub(in crate::plugin) fn capability_missing_other(
        &self,
        manifest: &Manifest,
        capability: &str,
        langs: &Languages,
    ) -> String {
        if capabilities::is_self_provided(manifest, capability) {
            crate::l10n::text(
                langs,
                "plugin-capability-self-only",
                &[("capability", capability.into())],
            )
        } else {
            self.capability_missing(capability, langs)
        }
    }

    fn capability_stopped(
        &self,
        provider: &PluginId,
        capability: &str,
        langs: &Languages,
    ) -> String {
        crate::l10n::text(
            langs,
            "plugin-capability-stopped",
            &[
                ("provider", provider.as_str().into()),
                ("capability", capability.into()),
            ],
        )
    }

    #[cfg(test)]
    pub(crate) fn capability_route_for_test(
        &self,
        consumer: &PluginId,
        capability: &str,
        provider: &PluginId,
        endpoint: Endpoint,
    ) {
        self.dependencies
            .change(|state| {
                state
                    .defaults
                    .insert(capability.to_owned(), provider.to_string());
                state.connections.push(Connection {
                    consumer: consumer.to_string(),
                    provider: provider.to_string(),
                    capability: capability.to_owned(),
                    ready: true,
                    ..Default::default()
                });
            })
            .unwrap();
        self.dependencies
            .routes
            .write()
            .unwrap()
            .insert((consumer.to_string(), capability.to_owned()), endpoint);
    }
}
