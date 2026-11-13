use std::collections::BTreeSet;

use wonton_protocol::{ConsumerCapabilityChoice, UninstallPreview};

use super::{
    InstallError, Languages, Manifest, PluginId, PluginManager, chosen, connection, declarations,
    high, reference, service,
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
        self.dependencies.change(|state| {
            state.defaults.insert(capability.to_owned(), id.to_string());
        })?;
        let mut failure: Option<InstallError> = None;
        for consumer in self.capability_consumers() {
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
            let connected = self
                .connect_capability(&consumer.manifest.id, provider, capability, langs)
                .await;
            let rebuilt = self
                .rebuild_capability_consumer(&consumer.manifest.id, langs)
                .await;
            let result = match (connected, rebuilt) {
                (Err(error), Err(cleanup)) => Err(error.with_note(langs, &cleanup.to_string())),
                (Err(error), _) | (_, Err(error)) => Err(error),
                _ => Ok(()),
            };
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

    pub async fn set_consumer_capability(
        &self,
        consumer: &PluginId,
        choice: ConsumerCapabilityChoice,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let _changing = self.dependencies.changing.lock().await;
        let record = self
            .store
            .load(consumer)?
            .ok_or_else(|| InstallError::not_installed(langs, consumer))?;
        let package = super::super::upgrade::installed_package(&record, langs)?;
        if !declarations(&package.manifest).any(|c| c == &choice.capability) {
            return Err(InstallError::Invalid(
                self.capability_missing(&choice.capability, langs),
            ));
        }
        // ADR-0061 第 9 节：提供方不能选自己当提供方。
        if choice.provider.as_deref() == Some(consumer.as_str()) {
            return Err(InstallError::Invalid(self.capability_missing_other(
                &package.manifest,
                &choice.capability,
                langs,
            )));
        }
        let providers = self.capability_providers();
        let mut next = self.dependencies.load()?;
        let entries = next.selected.entry(consumer.to_string()).or_default();
        match choice.provider {
            Some(id) => {
                entries.insert(choice.capability.clone(), id);
            }
            None => {
                entries.remove(&choice.capability);
            }
        }
        let provider = chosen(&next, &providers, consumer, &choice.capability, None);
        if provider.is_none()
            && (package.manifest.requires.contains(&choice.capability)
                || next
                    .selected
                    .get(consumer.as_str())
                    .is_some_and(|s| s.contains_key(&choice.capability)))
        {
            return Err(InstallError::Invalid(
                self.capability_missing(&choice.capability, langs),
            ));
        }
        self.dependencies.change(|state| {
            state.selected = next.selected;
        })?;
        let connected = match provider {
            Some(provider) => {
                self.connect_capability(consumer, provider, &choice.capability, langs)
                    .await
            }
            None => Ok(()),
        };
        let rebuilt = self.rebuild_capability_consumer(consumer, langs).await;
        match (connected, rebuilt) {
            (Err(error), Err(cleanup)) => return Err(error.with_note(langs, &cleanup.to_string())),
            (Err(error), _) | (_, Err(error)) => return Err(error),
            _ => {}
        }
        self.changed();
        Ok(())
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
                                    || connection(&state, &consumer.manifest.id, &p.id, capability)
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
                            .or_else(|| before.defaults.get(*capability))
                            .is_some_and(|p| removing.iter().any(|removed| removed.as_str() == p))
                    })
                    .map(|capability| (consumer.manifest.id.clone(), capability.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        self.dependencies.change(|state| {
            state
                .connections
                .retain(|c| c.consumer != id.as_str() && c.provider != id.as_str());
            state.selected.remove(id.as_str());
            for selected in state.selected.values_mut() {
                selected.retain(|_, p| !removing.iter().any(|removed| removed.as_str() == p));
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
        })?;
        let state = self.dependencies.load()?;
        let mut rebuilt = BTreeSet::new();
        let mut failure: Option<InstallError> = None;
        for (consumer, capability) in affected {
            if let Some(provider) = chosen(&state, &providers, &consumer, &capability, None)
                && (approve_connections
                    || !high(&provider.listed.manifest, &capability)
                    || connection(&state, &consumer, &provider.id, &capability).is_some())
                && let Err(error) = self
                    .connect_capability(&consumer, provider, &capability, langs)
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
                            .or_else(|| before.defaults.get(*capability))
                            .is_some_and(|p| p == manifest.id.as_str())
                            && service(manifest, capability).is_none()
                    })
                    .map(|capability| (consumer.manifest.id.clone(), capability.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        self.dependencies.change(|state| {
            state.connections.retain(|c| {
                c.provider != manifest.id.as_str() || service(manifest, &c.capability).is_some()
            });
            for selected in state.selected.values_mut() {
                selected.retain(|capability, id| {
                    id != manifest.id.as_str() || service(manifest, capability).is_some()
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
                            || connection(state, consumer, &p.id, capability).is_some())
                }) && chosen(state, &providers, consumer, capability, None)
                    .is_none_or(|selected| selected.id != provider.id)
                {
                    state
                        .selected
                        .entry(consumer.to_string())
                        .or_default()
                        .insert(capability.clone(), provider.id.to_string());
                }
            }
        })?;
        let state = self.dependencies.load()?;
        let mut rebuilt = BTreeSet::new();
        for (consumer, capability) in affected {
            if let Some(provider) = chosen(&state, &providers, &consumer, &capability, None)
                && (!high(&provider.listed.manifest, &capability)
                    || connection(&state, &consumer, &provider.id, &capability).is_some())
            {
                self.connect_capability(&consumer, provider, &capability, langs)
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
