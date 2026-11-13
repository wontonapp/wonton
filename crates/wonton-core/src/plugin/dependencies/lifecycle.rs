use super::{
    InstallError, Languages, Manifest, PluginId, PluginManager, PluginStatus, chosen, connection,
    declarations, high, reference, service,
};
use std::collections::BTreeMap;
use wonton_protocol::{PluginHint, PluginHintCategory};

impl PluginManager {
    pub(in crate::plugin) async fn prepare_dependencies(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let providers = self.capability_providers();
        let state = self.dependencies.load()?;
        for capability in declarations(manifest) {
            let required = manifest.requires.contains(capability);
            let Some(provider) = chosen(&state, &providers, &manifest.id, capability, None) else {
                if required {
                    return Err(InstallError::Conflict(
                        self.capability_missing_other(manifest, capability, langs),
                    ));
                }
                continue;
            };
            if self.store.load(&manifest.id)?.is_some()
                && high(&provider.listed.manifest, capability)
                && connection(&state, &manifest.id, &provider.id, capability).is_none()
            {
                if required {
                    return Err(InstallError::Conflict(
                        self.capability_missing_other(manifest, capability, langs),
                    ));
                }
                continue;
            }
            self.prepare_capability_connection(manifest, provider, capability, langs)
                .await?;
        }
        Ok(())
    }

    pub(in crate::plugin) async fn prepare_confirmed_dependencies(
        &self,
        manifest: &Manifest,
        targets: &BTreeMap<String, String>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let providers = self.capability_providers();
        for capability in declarations(manifest) {
            let Some(id) = targets.get(capability) else {
                if manifest.requires.contains(capability) {
                    return Err(InstallError::Conflict(
                        self.capability_missing_other(manifest, capability, langs),
                    ));
                }
                continue;
            };
            let provider = providers
                .iter()
                .find(|provider| {
                    provider.id.as_str() == id
                        && service(&provider.listed.manifest, capability).is_some()
                })
                .ok_or_else(|| {
                    InstallError::Conflict(self.capability_missing(capability, langs))
                })?;
            self.prepare_capability_connection(manifest, provider, capability, langs)
                .await?;
            self.dependencies.change(|state| {
                if chosen(state, &providers, &manifest.id, capability, None)
                    .is_none_or(|selected| selected.id != provider.id)
                {
                    state
                        .selected
                        .entry(manifest.id.to_string())
                        .or_default()
                        .insert(capability.clone(), id.clone());
                }
            })?;
        }
        Ok(())
    }

    async fn prepare_capability_connection(
        &self,
        manifest: &Manifest,
        provider: &super::Provider,
        capability: &str,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        match self
            .connect_capability(&manifest.id, provider, capability, langs)
            .await
        {
            Err(error)
                if manifest
                    .requires
                    .iter()
                    .any(|required| required == capability)
                    || matches!(error, InstallError::Storage(_)) =>
            {
                Err(error)
            }
            // Optional connection failures remain in the durable record and detail hints.
            _ => Ok(()),
        }
    }

    pub(in crate::plugin) async fn capabilities_installed(
        &self,
        manifest: &Manifest,
        connect_plugins: &[String],
        force: bool,
    ) -> Result<(), InstallError> {
        let providers = self.capability_providers();
        self.dependencies.change(|state| {
            if manifest
                .provides
                .as_ref()
                .is_some_and(|p| !p.capabilities.is_empty())
                && !state.providers.iter().any(|id| id == manifest.id.as_str())
            {
                state.providers.push(manifest.id.to_string());
            }
            for provider in &providers {
                if let Some(provides) = &provider.listed.manifest.provides {
                    for capability in provides.capabilities.keys() {
                        state
                            .defaults
                            .entry(capability.clone())
                            .or_insert_with(|| provider.id.to_string());
                    }
                }
            }
        })?;
        let langs = self.owner_languages();
        let mut failure: Option<InstallError> = None;
        for consumer in self.capability_consumers() {
            let state = self.dependencies.load()?;
            let vars_before = self.dependency_vars(&consumer.manifest)?;
            for capability in declarations(&consumer.manifest) {
                let Some(provider) =
                    chosen(&state, &providers, &consumer.manifest.id, capability, None)
                else {
                    continue;
                };
                let existing = connection(&state, &consumer.manifest.id, &provider.id, capability);
                let approved = existing.is_some()
                    || (provider.id == manifest.id
                        && (!high(&provider.listed.manifest, capability)
                            || connect_plugins.contains(&consumer.manifest.id.to_string())));
                if !approved
                    || !matches!(
                        provider.status,
                        PluginStatus::Running | PluginStatus::Degraded { .. }
                    )
                    || !service(&provider.listed.manifest, capability)
                        .is_some_and(|s| provider.serving.contains(&s.service))
                {
                    continue;
                }
                if (force && provider.id == manifest.id)
                    || existing.is_none_or(|c| !c.ready || c.problem.is_some())
                {
                    match self
                        .connect_capability(&consumer.manifest.id, provider, capability, &langs)
                        .await
                    {
                        Ok(()) => {}
                        Err(error)
                            if consumer.manifest.id != manifest.id
                                || consumer.manifest.requires.contains(capability)
                                || matches!(error, InstallError::Storage(_)) =>
                        {
                            failure = Some(match failure {
                                Some(previous) => error.with_note(&langs, &previous.to_string()),
                                None => error,
                            });
                        }
                        Err(_) => {}
                    }
                }
            }
            if (vars_before != self.dependency_vars(&consumer.manifest)?
                || state
                    .connections
                    .iter()
                    .any(|c| c.consumer == consumer.manifest.id.as_str() && c.rebuild))
                && let Err(error) = self
                    .rebuild_connected_consumer(&consumer.manifest, &state, &langs)
                    .await
            {
                failure = Some(match failure {
                    Some(previous) => error.with_note(&langs, &previous.to_string()),
                    None => error,
                });
            }
        }
        self.changed();
        failure.map_or(Ok(()), Err)
    }

    async fn rebuild_connected_consumer(
        &self,
        manifest: &Manifest,
        before: &super::State,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        self.dependencies.change(|state| {
            for c in state
                .connections
                .iter_mut()
                .filter(|c| c.consumer == manifest.id.as_str())
            {
                c.rebuild = true;
            }
        })?;
        let result = self.rebuild_capability_consumer(&manifest.id, langs).await;
        self.dependencies.change(|state| {
            for c in state
                .connections
                .iter_mut()
                .filter(|c| c.consumer == manifest.id.as_str())
            {
                match &result {
                    Ok(()) => c.rebuild = false,
                    Err(error) => {
                        c.ready = before.connections.iter().any(|previous| {
                            previous.consumer == c.consumer
                                && previous.provider == c.provider
                                && previous.capability == c.capability
                                && previous.ready
                        });
                        c.problem = Some(error.to_string());
                    }
                }
            }
        })?;
        result
    }

    pub(in crate::plugin) async fn rebuild_capability_consumer(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let pending_rebuild = self
            .dependencies
            .load()?
            .connections
            .iter()
            .any(|c| c.consumer == id.as_str() && c.rebuild);
        if !pending_rebuild
            && !matches!(
                self.status_of(id),
                Some(PluginStatus::Running | PluginStatus::Degraded { .. })
            )
        {
            return Ok(());
        }
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        let package = super::super::upgrade::installed_package(&record, langs)?;
        let lan = self.lan_grant(&package, record.lan_webui, langs).await?;
        let mut ctx = self.rebuild_context(&package, &record, lan, None).await?;
        self.fill_bot_vars(&package, &mut ctx, langs).await?;
        let plan = super::super::render::render(&package, &ctx, langs)?;
        let deployed = async {
            self.replace_containers(&ctx, plan, super::super::health_gate::Gate::StartPeriod)
                .await?;
            self.reconcile_running(&package, &record).await
        }
        .await;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins
            .get_mut(id)
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        let listed = entry
            .listed
            .clone()
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        let result = match deployed {
            Ok(deployed) => {
                *entry = super::super::entry::Entry::running(listed, deployed);
                Ok(())
            }
            Err(error) => {
                *entry = super::super::entry::Entry::containers_failed(listed, error.to_string());
                Err(error)
            }
        };
        drop(plugins);
        self.changed();
        result
    }

    pub(in crate::plugin) fn dependency_hints(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Vec<PluginHint> {
        let providers = self.capability_providers();
        let state = match self.dependencies.load() {
            Ok(state) => state,
            Err(error) => {
                return declarations(manifest)
                    .map(|capability| PluginHint {
                        category: PluginHintCategory::DependencyStopped,
                        detail: Some(error.to_string()),
                        provider: None,
                        capability: Some(capability.clone()),
                    })
                    .collect();
            }
        };
        declarations(manifest)
            .filter_map(|capability| {
                let provider = chosen(&state, &providers, &manifest.id, capability, None);
                let required = manifest.requires.contains(capability);
                let (category, detail) = match provider {
                    None => (
                        if required {
                            PluginHintCategory::DependencyStopped
                        } else {
                            PluginHintCategory::WantsUnsatisfied
                        },
                        None,
                    ),
                    Some(p) => {
                        let connected = connection(&state, &manifest.id, &p.id, capability);
                        match connected {
                            None => (PluginHintCategory::ProviderNotConnected, None),
                            Some(c) if c.problem.is_some() => {
                                (PluginHintCategory::DependencyStopped, c.problem.clone())
                            }
                            Some(_)
                                if !matches!(
                                    p.status,
                                    PluginStatus::Running | PluginStatus::Degraded { .. }
                                ) || !service(&p.listed.manifest, capability)
                                    .is_some_and(|s| p.serving.contains(&s.service)) =>
                            {
                                (PluginHintCategory::DependencyStopped, None)
                            }
                            Some(_) => return None,
                        }
                    }
                };
                Some(PluginHint {
                    category,
                    detail,
                    provider: provider.map(|p| reference(p, capability, langs)),
                    capability: Some(capability.clone()),
                })
            })
            .collect()
    }

    pub(in crate::plugin) fn dependency_problem(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Option<String> {
        self.dependency_hints(manifest, langs)
            .into_iter()
            .find(|hint| {
                hint.capability
                    .as_ref()
                    .is_some_and(|capability| manifest.requires.contains(capability))
            })
            .map(|hint| {
                hint.detail.unwrap_or_else(|| match hint.provider {
                    Some(p) => crate::l10n::text(
                        langs,
                        "plugin-capability-stopped",
                        &[
                            ("provider", p.title.into()),
                            ("capability", hint.capability.unwrap_or_default().into()),
                        ],
                    ),
                    None => self
                        .capability_missing(hint.capability.as_deref().unwrap_or_default(), langs),
                })
            })
    }

    pub(in crate::plugin) async fn start_dependency_providers(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let providers = self.capability_providers();
        let state = self.dependencies.load()?;
        for capability in manifest.requires.iter().filter(|c| super::generic(c)) {
            let provider = chosen(&state, &providers, &manifest.id, capability, None)
                .ok_or_else(|| {
                    InstallError::Conflict(
                        self.capability_missing_other(manifest, capability, langs),
                    )
                })?;
            if connection(&state, &manifest.id, &provider.id, capability).is_none() {
                return Err(InstallError::Conflict(
                    self.capability_missing(capability, langs),
                ));
            }
            if provider.status == PluginStatus::Stopped
                || (matches!(provider.status, PluginStatus::Degraded { .. })
                    && !service(&provider.listed.manifest, capability)
                        .is_some_and(|s| provider.serving.contains(&s.service)))
            {
                Box::pin(self.start(&provider.id)).await?;
            }
            let current = self.capability_providers();
            let provider = current
                .iter()
                .find(|p| p.id == provider.id)
                .ok_or_else(|| {
                    InstallError::Conflict(self.capability_missing(capability, langs))
                })?;
            if !matches!(
                provider.status,
                PluginStatus::Running | PluginStatus::Degraded { .. }
            ) || !service(&provider.listed.manifest, capability)
                .is_some_and(|s| provider.serving.contains(&s.service))
            {
                return Err(InstallError::UpstreamUnavailable(self.capability_stopped(
                    &provider.id,
                    capability,
                    langs,
                )));
            }
            let current = self.dependencies.load()?;
            if connection(&current, &manifest.id, &provider.id, capability)
                .is_some_and(|c| c.problem.is_some())
            {
                self.connect_capability(&manifest.id, provider, capability, langs)
                    .await?;
            }
        }
        Ok(())
    }
}
