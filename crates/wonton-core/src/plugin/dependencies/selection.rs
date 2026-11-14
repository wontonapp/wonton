use std::collections::BTreeSet;

use wonton_protocol::{BotRef, ConsumerCapabilityChoice, UninstallPreview};

use super::binding::consumer_key;
use super::{
    Connection, InstallError, Languages, Manifest, PluginId, PluginManager, State, chosen,
    connection, connection_to, declarations, high, reference, service,
};

impl PluginManager {
    pub async fn set_capability_default(
        &self,
        capability: &str,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let _changing = self.dependencies.changing.lock().await;
        let providers = self.capability_providers();
        let provider = providers
            .iter()
            .find(|p| p.id == *id && service(&p.listed.manifest, capability).is_some())
            .ok_or_else(|| InstallError::Invalid(self.capability_missing(capability, langs)))?;
        let before = self.dependencies.load()?;
        let consumers = self.capability_consumers();
        let mut affected = Vec::with_capacity(consumers.len());
        let mut pending = Vec::new();
        for consumer in consumers {
            // ADR-0061 第 9 节：提供方自己不能连到自己。
            if consumer.manifest.id == *id
                || !declarations(&consumer.manifest).any(|c| c == capability)
                || before
                    .selected
                    .get(consumer.manifest.id.as_str())
                    .is_some_and(|s| s.contains_key(capability))
            {
                continue;
            }
            let bots = self.default_inheriting_bots(
                &consumer.manifest,
                capability,
                &before,
                &providers,
                high(&provider.listed.manifest, capability),
                langs,
            )?;
            for bot in std::iter::once(None).chain(bots.iter().map(Some)) {
                if let Some(connection) = self.pending_default_connection(
                    &before,
                    &consumer.manifest.id,
                    capability,
                    id,
                    bot,
                    langs,
                )? {
                    pending.push(connection);
                }
            }
            affected.push((consumer, bots));
        }
        self.dependencies.change(|state| {
            state.defaults.insert(capability.to_owned(), id.to_string());
            state.connections.extend(pending);
        })?;
        let mut failure: Option<InstallError> = None;
        for (consumer, bots) in affected {
            let result = self
                .connect_default_consumer(&consumer.manifest, capability, provider, bots, langs)
                .await;
            if let Err(error) = result {
                failure = Some(match failure {
                    Some(previous) => error.with_note(langs, &previous.to_string()),
                    None => error,
                });
            }
        }
        self.changed();
        failure.map_or(Ok(()), Err)
    }

    fn pending_default_connection(
        &self,
        state: &State,
        consumer: &PluginId,
        capability: &str,
        provider: &PluginId,
        bot: Option<&BotRef>,
        langs: &Languages,
    ) -> Result<Option<Connection>, InstallError> {
        let target =
            self.capability_target_bot(state, consumer, provider, capability, bot, langs)?;
        let target_name = target.as_ref().map(|bot| bot.bot.as_str());
        Ok(
            connection_to(state, consumer, provider, capability, bot, target_name)
                .is_none()
                .then(|| {
                    Connection::pending(
                        consumer.as_str(),
                        bot.map(|bot| bot.bot.as_str()),
                        provider.as_str(),
                        target_name,
                        capability,
                    )
                }),
        )
    }

    fn default_inheriting_bots(
        &self,
        manifest: &Manifest,
        capability: &str,
        before: &State,
        providers: &[super::Provider],
        high_provider: bool,
        langs: &Languages,
    ) -> Result<Vec<BotRef>, InstallError> {
        let mut affected = Vec::new();
        for bot in self.active_capability_bots(&manifest.id)? {
            if before
                .selected
                .get(&consumer_key(manifest.id.as_str(), &bot.bot))
                .is_some_and(|choices| choices.contains_key(capability))
            {
                continue;
            }
            if high_provider {
                let Some(previous) =
                    chosen(before, providers, &manifest.id, capability, Some(&bot))
                else {
                    continue;
                };
                if !before.connections.iter().any(|c| {
                    c.consumer == manifest.id.as_str()
                        && c.consumer_bot.as_deref() == Some(&bot.bot)
                        && c.provider == previous.id.as_str()
                        && c.capability == capability
                }) {
                    continue;
                }
                let target = match self.capability_target_bot(
                    before,
                    &manifest.id,
                    &previous.id,
                    capability,
                    Some(&bot),
                    langs,
                ) {
                    Ok(target) => target,
                    Err(InstallError::UpstreamUnavailable(_)) => continue,
                    Err(error) => return Err(error),
                };
                if connection_to(
                    before,
                    &manifest.id,
                    &previous.id,
                    capability,
                    Some(&bot),
                    target.as_ref().map(|target| target.bot.as_str()),
                )
                .is_none()
                {
                    continue;
                }
            }
            affected.push(bot);
        }
        Ok(affected)
    }

    async fn connect_default_consumer(
        &self,
        manifest: &Manifest,
        capability: &str,
        provider: &super::Provider,
        bots: Vec<BotRef>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let mut connected = self
            .connect_capability(&manifest.id, provider, capability, langs, None)
            .await;
        for bot in bots {
            if let Err(error) = self
                .connect_capability(&manifest.id, provider, capability, langs, Some(&bot))
                .await
            {
                connected = Err(match connected {
                    Err(previous) => previous.with_note(langs, &error.to_string()),
                    Ok(()) => error,
                });
            }
        }
        let rebuilt = self.rebuild_capability_consumer(&manifest.id, langs).await;
        match (connected, rebuilt) {
            (Err(error), Err(cleanup)) => Err(error.with_note(langs, &cleanup.to_string())),
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    pub async fn set_consumer_capability(
        &self,
        consumer: &PluginId,
        choice: ConsumerCapabilityChoice,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        self.set_capability_binding(consumer, None, choice, None, langs)
            .await
    }

    pub(in crate::plugin) fn capability_dependents(
        &self,
        provider: &PluginId,
        replacement: Option<&Manifest>,
        langs: &Languages,
    ) -> Result<Vec<wonton_protocol::CapabilityProviderRef>, InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let mut dependents = Vec::new();
        for consumer in self.capability_consumers() {
            let blocked = consumer
                .manifest
                .requires
                .iter()
                .filter(|c| super::generic(c))
                .any(|capability| {
                    chosen(&state, &providers, &consumer.manifest.id, capability, None)
                        .is_some_and(|p| p.id == *provider)
                        && !replacement.is_some_and(|m| service(m, capability).is_some())
                        && !providers.iter().any(|p| {
                            p.id != *provider
                                // 自己提供不算替代：requires 要的是别的插件（ADR-0061 第 9 节）。
                                && p.id != consumer.manifest.id
                                && service(&p.listed.manifest, capability).is_some()
                                && (replacement.is_none()
                                    || !high(&p.listed.manifest, capability)
                                    || connection(&state, &consumer.manifest.id, &p.id, capability, None)
                                        .is_some())
                        })
                });
            if blocked {
                dependents.push(wonton_protocol::CapabilityProviderRef {
                    id: consumer.manifest.id.to_string(),
                    title: langs.pick_owned(&consumer.manifest.title),
                    high_privilege: false,
                });
            }
        }
        Ok(dependents)
    }

    pub(in crate::plugin) fn check_capability_removal(
        &self,
        id: &PluginId,
        replacement: Option<&Manifest>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let plugins = self.capability_dependents(id, replacement, langs)?;
        if plugins.is_empty() {
            return Ok(());
        }
        let titles: Vec<_> = plugins.iter().map(|p| p.title.as_str()).collect();
        Err(InstallError::InUse {
            message: crate::l10n::text(
                langs,
                "plugin-capability-in-use",
                &[("plugins", crate::l10n::list(langs, &titles).into())],
            ),
            plugins,
        })
    }

    pub fn uninstall_preview(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<UninstallPreview, InstallError> {
        if self.failed_install(id) {
            return Ok(UninstallPreview {
                id: id.to_string(),
                unused_providers: Vec::new(),
            });
        }
        if !self.is_installed(id) {
            return Err(InstallError::not_installed(langs, id));
        }
        self.check_capability_removal(id, None, langs)?;
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let consumers = self.capability_consumers();
        let selected_uses = |consumer: &Manifest, provider: &PluginId| {
            declarations(consumer).any(|capability| {
                chosen(&state, &providers, &consumer.id, capability, None)
                    .is_some_and(|selected| selected.id == *provider)
            })
        };
        let target = consumers
            .iter()
            .find(|consumer| consumer.manifest.id == *id);
        let unused_providers = providers
            .iter()
            .filter(|provider| {
                target.is_some_and(|consumer| selected_uses(&consumer.manifest, &provider.id))
                    && provider.listed.manifest.kind != super::PluginKind::System
                    && !consumers.iter().any(|consumer| {
                        consumer.manifest.id != *id
                            && selected_uses(&consumer.manifest, &provider.id)
                    })
            })
            .map(|p| reference(p, "", langs))
            .collect();
        Ok(UninstallPreview {
            id: id.to_string(),
            unused_providers,
        })
    }

    pub(in crate::plugin) async fn remove_capability_records(
        &self,
        id: &PluginId,
        removing: &[PluginId],
        approve_connections: bool,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let before = self.dependencies.load()?;
        let providers: Vec<_> = self
            .capability_providers()
            .into_iter()
            .filter(|p| !removing.contains(&p.id))
            .collect();
        let consumers = self.capability_consumers();
        let affected: Vec<_> = consumers
            .iter()
            .filter(|c| !removing.contains(&c.manifest.id))
            .flat_map(|consumer| {
                declarations(&consumer.manifest)
                    .filter(|capability| {
                        before
                            .selected
                            .get(consumer.manifest.id.as_str())
                            .and_then(|s| s.get(*capability))
                            .and_then(super::binding::Binding::provider)
                            .or_else(|| before.defaults.get(*capability).map(String::as_str))
                            .is_some_and(|p| removing.iter().any(|removed| removed.as_str() == p))
                    })
                    .map(|capability| (consumer.manifest.id.clone(), capability.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut rebuilt: BTreeSet<_> = before
            .connections
            .iter()
            .filter(|c| {
                c.consumer_bot.is_some() && removing.iter().any(|id| id.as_str() == c.provider)
            })
            .filter_map(|c| PluginId::parse(&c.consumer).ok())
            .filter(|consumer| !removing.contains(consumer))
            .collect();
        self.prune_removed_capability_state(id, removing, &providers)?;
        let state = self.dependencies.load()?;
        let mut failure: Option<InstallError> = None;
        for (consumer, capability) in affected {
            if let Some(provider) = chosen(&state, &providers, &consumer, &capability, None)
                && (approve_connections
                    || !high(&provider.listed.manifest, &capability)
                    || connection(&state, &consumer, &provider.id, &capability, None).is_some())
                && let Err(error) = self
                    .connect_capability(&consumer, provider, &capability, langs, None)
                    .await
            {
                failure = Some(match failure {
                    Some(previous) => error.with_note(langs, &previous.to_string()),
                    None => error,
                });
            }
            rebuilt.insert(consumer);
        }
        for consumer in rebuilt {
            if let Err(error) = self.rebuild_capability_consumer(&consumer, langs).await {
                failure = Some(match failure {
                    Some(previous) => error.with_note(langs, &previous.to_string()),
                    None => error,
                });
            }
        }
        self.changed();
        failure.map_or(Ok(()), Err)
    }

    fn prune_removed_capability_state(
        &self,
        id: &PluginId,
        removing: &[PluginId],
        providers: &[super::Provider],
    ) -> Result<(), InstallError> {
        self.dependencies.change(|state| {
            state
                .connections
                .retain(|c| c.consumer != id.as_str() && c.provider != id.as_str());
            state.handoff_consents.retain(|consent| {
                !removing.iter().any(|id| {
                    id.as_str() == consent.from.plugin_id || id.as_str() == consent.target.plugin_id
                })
            });
            state.selected.retain(|scope, _| {
                scope != id.as_str()
                    && !scope
                        .strip_prefix(id.as_str())
                        .is_some_and(|suffix| suffix.starts_with('/'))
            });
            for selected in state.selected.values_mut() {
                selected.retain(|_, p| {
                    !removing
                        .iter()
                        .any(|removed| Some(removed.as_str()) == p.provider())
                });
            }
            state.providers.retain(|p| p != id.as_str());
            for (capability, selected) in &mut state.defaults {
                if removing.iter().any(|removed| removed.as_str() == selected) {
                    *selected = providers
                        .iter()
                        .find(|p| service(&p.listed.manifest, capability).is_some())
                        .map_or_else(String::new, |p| p.id.to_string());
                }
            }
            state.defaults.retain(|_, id| !id.is_empty());
            state
                .provider_tokens
                .retain(|key, _| !key.ends_with(&format!("/{id}")));
            for removed in removing {
                state
                    .provider_tokens
                    .retain(|key, _| !key.ends_with(&format!("/{removed}")));
            }
        })
    }

    pub(in crate::plugin) async fn prune_provider_capabilities(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let providers = self.capability_providers();
        let consumers = self.capability_consumers();
        let before = self.dependencies.load()?;
        let affected: Vec<_> = consumers
            .iter()
            .flat_map(|consumer| {
                declarations(&consumer.manifest)
                    .filter(|capability| {
                        before
                            .selected
                            .get(consumer.manifest.id.as_str())
                            .and_then(|s| s.get(*capability))
                            .and_then(super::binding::Binding::provider)
                            .or_else(|| before.defaults.get(*capability).map(String::as_str))
                            .is_some_and(|p| p == manifest.id.as_str())
                            && service(manifest, capability).is_none()
                    })
                    .map(|capability| (consumer.manifest.id.clone(), capability.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut rebuilt: BTreeSet<_> = before
            .connections
            .iter()
            .filter(|c| {
                c.consumer_bot.is_some()
                    && c.provider == manifest.id.as_str()
                    && service(manifest, &c.capability).is_none()
            })
            .filter_map(|c| PluginId::parse(&c.consumer).ok())
            .collect();
        self.dependencies.change(|state| {
            state.connections.retain(|c| {
                c.provider != manifest.id.as_str() || service(manifest, &c.capability).is_some()
            });
            for selected in state.selected.values_mut() {
                selected.retain(|capability, id| {
                    id.provider() != Some(manifest.id.as_str())
                        || service(manifest, capability).is_some()
                });
            }
            for (capability, id) in &mut state.defaults {
                if id == manifest.id.as_str() && service(manifest, capability).is_none() {
                    *id = providers
                        .iter()
                        .find(|p| service(&p.listed.manifest, capability).is_some())
                        .map_or_else(String::new, |p| p.id.to_string());
                }
            }
            state.defaults.retain(|_, id| !id.is_empty());
            // 只删这个插件不再提供的能力的令牌；别的提供方的令牌不动。
            state
                .provider_tokens
                .retain(|key, _| match key.split_once('/') {
                    Some((capability, provider)) => {
                        provider != manifest.id.as_str() || service(manifest, capability).is_some()
                    }
                    None => true,
                });
            for (consumer, capability) in &affected {
                if let Some(provider) = providers.iter().find(|p| {
                    service(&p.listed.manifest, capability).is_some()
                        && (!high(&p.listed.manifest, capability)
                            || connection(state, consumer, &p.id, capability, None).is_some())
                }) && chosen(state, &providers, consumer, capability, None)
                    .is_none_or(|selected| selected.id != provider.id)
                {
                    state
                        .selected
                        .entry(consumer.to_string())
                        .or_default()
                        .insert(
                            capability.clone(),
                            super::binding::Binding::Plugin(provider.id.to_string()),
                        );
                }
            }
        })?;
        let state = self.dependencies.load()?;
        for (consumer, capability) in affected {
            if let Some(provider) = chosen(&state, &providers, &consumer, &capability, None)
                && (!high(&provider.listed.manifest, &capability)
                    || connection(&state, &consumer, &provider.id, &capability, None).is_some())
            {
                self.connect_capability(&consumer, provider, &capability, langs, None)
                    .await?;
            }
            rebuilt.insert(consumer);
        }
        for consumer in rebuilt {
            self.rebuild_capability_consumer(&consumer, langs).await?;
        }
        Ok(())
    }
}
