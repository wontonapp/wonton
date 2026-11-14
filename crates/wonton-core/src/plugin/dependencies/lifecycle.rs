use super::{
    InstallError, Languages, Manifest, PluginId, PluginManager, PluginStatus, Provider, State,
    chosen, connection, connection_to, declarations, high, reference, service,
};
use std::collections::{BTreeMap, BTreeSet};
use wonton_protocol::{BotRef, PluginHint, PluginHintCategory};

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
                && connection(&state, &manifest.id, &provider.id, capability, None).is_none()
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
                        .insert(
                            capability.clone(),
                            super::binding::Binding::Plugin(id.clone()),
                        );
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
            .connect_capability(&manifest.id, provider, capability, langs, None)
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
            // 恢复包含各 Bot 的重建状态机，不能把整棵状态机叠进安装或巡检的调用栈。
            if let Err(error) = Box::pin(self.recover_capability_consumer(
                &consumer.manifest,
                manifest,
                connect_plugins,
                force,
                &langs,
            ))
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

    async fn recover_capability_consumer(
        &self,
        manifest: &Manifest,
        installed: &Manifest,
        connect_plugins: &[String],
        force: bool,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let providers = self.capability_providers();
        let before = self.dependencies.load()?;
        let vars_before = self.dependency_vars(manifest, None)?;
        let bots = self.store.bots().load(&manifest.id)?;
        let bot_vars_before = bots
            .bots
            .keys()
            .map(|bot| Ok((bot.clone(), self.dependency_vars(manifest, Some(bot))?)))
            .collect::<Result<BTreeMap<_, _>, InstallError>>()?;
        let mut failure: Option<InstallError> = None;
        for capability in declarations(manifest) {
            let Some(provider) = chosen(&before, &providers, &manifest.id, capability, None) else {
                continue;
            };
            let existing = connection(&before, &manifest.id, &provider.id, capability, None);
            let approved = existing.is_some()
                || (provider.id == installed.id
                    && (!high(&provider.listed.manifest, capability)
                        || connect_plugins.contains(&manifest.id.to_string())));
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
            if ((force && provider.id == installed.id)
                || existing.is_none_or(|c| !c.ready || c.problem.is_some()))
                && let Err(error) = self
                    .connect_capability(&manifest.id, provider, capability, langs, None)
                    .await
                && (manifest.id != installed.id
                    || manifest.requires.contains(capability)
                    || matches!(error, InstallError::Storage(_)))
            {
                failure = Some(match failure {
                    Some(previous) => error.with_note(langs, &previous.to_string()),
                    None => error,
                });
            }
        }
        for bot in bots.bots.keys() {
            if let Err(error) = self
                .prepare_bot_dependencies(manifest, bot, langs, force.then_some(&installed.id))
                .await
            {
                failure = Some(match failure {
                    Some(previous) => error.with_note(langs, &previous.to_string()),
                    None => error,
                });
            }
        }
        let rebuilt = self
            .rebuild_changed_capability_scopes(
                manifest,
                &before,
                vars_before,
                bot_vars_before,
                langs,
            )
            .await;
        match (failure, rebuilt) {
            (Some(error), Err(cleanup)) => Err(error.with_note(langs, &cleanup.to_string())),
            (Some(error), _) | (None, Err(error)) => Err(error),
            (None, Ok(())) => Ok(()),
        }
    }

    async fn rebuild_changed_capability_scopes(
        &self,
        manifest: &Manifest,
        before: &super::State,
        vars_before: BTreeMap<String, String>,
        bot_vars_before: BTreeMap<String, BTreeMap<String, String>>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let pending = self.dependencies.load()?;
        if vars_before != self.dependency_vars(manifest, None)?
            || pending.connections.iter().any(|c| {
                c.consumer == manifest.id.as_str() && c.consumer_bot.is_none() && c.rebuild
            })
        {
            return self
                .rebuild_connected_consumer(manifest, before, langs)
                .await;
        }
        let mut failure: Option<InstallError> = None;
        for (bot, vars) in bot_vars_before {
            if (vars != self.dependency_vars(manifest, Some(&bot))?
                || pending.connections.iter().any(|c| {
                    c.consumer == manifest.id.as_str()
                        && c.consumer_bot.as_deref() == Some(bot.as_str())
                        && c.rebuild
                }))
                && let Err(error) = self.rebuild_capability_bot(&manifest.id, &bot, langs).await
            {
                failure = Some(match failure {
                    Some(previous) => error.with_note(langs, &previous.to_string()),
                    None => error,
                });
            }
        }
        failure.map_or(Ok(()), Err)
    }

    async fn rebuild_connected_consumer(
        &self,
        manifest: &Manifest,
        before: &super::State,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if let Some(recovery) = self.lifecycle_status(&manifest.id)?.recovery {
            return Err(InstallError::UpstreamUnavailable(recovery.reason));
        }
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
                        c.ready = before
                            .connections
                            .iter()
                            .any(|previous| previous.same_pair(c) && previous.ready);
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
        if !self.begin_rebuild(id) {
            return Err(InstallError::Conflict(crate::l10n::text(
                langs,
                "plugin-install-conflict-rebuilding",
                &[("id", id.to_string().into())],
            )));
        }
        let result = self.rebuild_capability_consumer_inner(id, langs).await;
        self.lan.end(id);
        result
    }

    pub(in crate::plugin) async fn rebuild_capability_bot(
        &self,
        id: &PluginId,
        bot: &str,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        if record.lifecycle.desired == wonton_protocol::PluginDesiredState::Stopped {
            return Ok(());
        }
        if let Some(recovery) = self.lifecycle_status(id)?.recovery {
            return Err(InstallError::UpstreamUnavailable(recovery.reason));
        }
        let before = self.dependencies.load()?;
        let pending = before.connections.iter().any(|c| {
            c.consumer == id.as_str() && c.consumer_bot.as_deref() == Some(bot) && c.rebuild
        });
        if !pending
            && !matches!(
                self.status_of(id),
                Some(PluginStatus::Running | PluginStatus::Degraded { .. })
            )
        {
            return Ok(());
        }
        let package = super::super::upgrade::installed_package(&record, langs)?;
        self.dependencies.change(|state| {
            for c in state
                .connections
                .iter_mut()
                .filter(|c| c.consumer == id.as_str() && c.consumer_bot.as_deref() == Some(bot))
            {
                c.rebuild = true;
            }
        })?;
        let result = if package.manifest.bots.is_none() {
            self.rebuild_capability_consumer(id, langs).await
        } else if self.begin_rebuild(id) {
            let result = self.rebuild_bot(id, bot).await;
            self.lan.end(id);
            result
        } else {
            Err(InstallError::Conflict(crate::l10n::text(
                langs,
                "plugin-install-conflict-rebuilding",
                &[("id", id.to_string().into())],
            )))
        };
        self.dependencies.change(|state| {
            for c in state
                .connections
                .iter_mut()
                .filter(|c| c.consumer == id.as_str() && c.consumer_bot.as_deref() == Some(bot))
            {
                match &result {
                    Ok(()) => c.rebuild = false,
                    Err(error) => {
                        c.ready = before
                            .connections
                            .iter()
                            .any(|previous| previous.same_pair(c) && previous.ready);
                        c.problem = Some(error.to_string());
                    }
                }
            }
        })?;
        self.changed();
        result
    }

    async fn rebuild_capability_consumer_inner(
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
        if record.lifecycle.desired == wonton_protocol::PluginDesiredState::Stopped {
            return Ok(());
        }
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
        let (state, bots) = match self.dependencies.load().and_then(|state| {
            self.active_capability_bots(&manifest.id)
                .map(|bots| (state, bots))
        }) {
            Ok(scope) => scope,
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
        let mut hints: Vec<PluginHint> = Vec::new();
        for capability in declarations(manifest) {
            for bot in std::iter::once(None).chain(bots.iter().map(Some)) {
                let provider = chosen(&state, &providers, &manifest.id, capability, bot);
                if let Some(hint) = self
                    .capability_dependency_hint(&state, manifest, capability, provider, bot, langs)
                    && !hints.iter().any(|previous| {
                        previous.category == hint.category
                            && previous.detail == hint.detail
                            && previous.capability == hint.capability
                            && previous.provider.as_ref().map(|p| &p.id)
                                == hint.provider.as_ref().map(|p| &p.id)
                    })
                {
                    hints.push(hint);
                }
            }
        }
        hints
    }

    fn capability_dependency_hint(
        &self,
        state: &State,
        manifest: &Manifest,
        capability: &str,
        provider: Option<&Provider>,
        bot: Option<&BotRef>,
        langs: &Languages,
    ) -> Option<PluginHint> {
        let (category, detail) = match provider {
            None => (
                if manifest.requires.iter().any(|name| name == capability) {
                    PluginHintCategory::DependencyStopped
                } else {
                    PluginHintCategory::WantsUnsatisfied
                },
                None,
            ),
            Some(provider) => match self.capability_dependency_connection(
                state,
                &manifest.id,
                provider,
                capability,
                bot,
                langs,
            ) {
                Err(error) => (
                    PluginHintCategory::DependencyStopped,
                    Some(error.to_string()),
                ),
                Ok(None) => (PluginHintCategory::ProviderNotConnected, None),
                Ok(Some(c)) if c.problem.is_some() => {
                    (PluginHintCategory::DependencyStopped, c.problem.clone())
                }
                Ok(Some(c))
                    if !c.ready
                        || !matches!(
                            provider.status,
                            PluginStatus::Running | PluginStatus::Degraded { .. }
                        )
                        || !service(&provider.listed.manifest, capability)
                            .is_some_and(|spec| provider.serving.contains(&spec.service)) =>
                {
                    (PluginHintCategory::DependencyStopped, None)
                }
                Ok(Some(_)) => return None,
            },
        };
        Some(PluginHint {
            category,
            detail,
            provider: provider.map(|p| reference(p, capability, langs)),
            capability: Some(capability.to_owned()),
        })
    }

    fn capability_dependency_connection<'a>(
        &self,
        state: &'a State,
        consumer: &PluginId,
        provider: &Provider,
        capability: &str,
        bot: Option<&BotRef>,
        langs: &Languages,
    ) -> Result<Option<&'a super::Connection>, InstallError> {
        let target =
            self.capability_target_bot(state, consumer, &provider.id, capability, bot, langs)?;
        let target_name = target.as_ref().map(|bot| bot.bot.as_str());
        Ok(
            connection_to(state, consumer, &provider.id, capability, bot, target_name).or_else(
                || {
                    (bot.is_some() && !high(&provider.listed.manifest, capability))
                        .then(|| {
                            connection_to(
                                state,
                                consumer,
                                &provider.id,
                                capability,
                                None,
                                target_name,
                            )
                        })
                        .flatten()
                },
            ),
        )
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
        manual: bool,
    ) -> Result<(), InstallError> {
        let mut providers = self.capability_providers();
        let state = self.dependencies.load()?;
        let bots = self.active_capability_bots(&manifest.id)?;
        let mut checked = BTreeMap::<usize, (bool, BTreeSet<&str>)>::new();
        for capability in manifest.requires.iter().filter(|c| super::generic(c)) {
            for bot in std::iter::once(None).chain(bots.iter().map(Some)) {
                let provider = chosen(&state, &providers, &manifest.id, capability, bot)
                    .ok_or_else(|| {
                        InstallError::Conflict(
                            self.capability_missing_other(manifest, capability, langs),
                        )
                    })?;
                if self
                    .capability_dependency_connection(
                        &state,
                        &manifest.id,
                        provider,
                        capability,
                        bot,
                        langs,
                    )?
                    .is_none()
                {
                    return Err(InstallError::Conflict(
                        self.capability_missing(capability, langs),
                    ));
                }
                let index = providers
                    .iter()
                    .position(|p| p.id == provider.id)
                    .expect("选中的提供方来自同一列表");
                let (started, capabilities) = checked.entry(index).or_default();
                if capabilities.insert(capability.as_str()) {
                    providers[index] = self
                        .start_required_capability_provider(
                            &providers[index],
                            capability,
                            langs,
                            manual,
                            started,
                        )
                        .await?;
                }
                let provider = &providers[index];
                let current = self.dependencies.load()?;
                if self
                    .capability_dependency_connection(
                        &current,
                        &manifest.id,
                        provider,
                        capability,
                        bot,
                        langs,
                    )?
                    .is_some_and(|c| !c.ready || c.problem.is_some())
                {
                    self.connect_capability(&manifest.id, provider, capability, langs, bot)
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn start_required_capability_provider(
        &self,
        provider: &Provider,
        capability: &str,
        langs: &Languages,
        manual: bool,
        started: &mut bool,
    ) -> Result<Provider, InstallError> {
        if !*started
            && !self.explicitly_stopped(&provider.id)?
            && (manual || !self.recovery_pending(&provider.id))
            && (provider.status == PluginStatus::Stopped
                || (matches!(provider.status, PluginStatus::Degraded { .. })
                    && !service(&provider.listed.manifest, capability)
                        .is_some_and(|spec| provider.serving.contains(&spec.service))))
        {
            *started = true;
            Box::pin(self.start(&provider.id)).await?;
        }
        let current = self
            .capability_providers()
            .into_iter()
            .find(|p| p.id == provider.id)
            .ok_or_else(|| InstallError::Conflict(self.capability_missing(capability, langs)))?;
        if !matches!(
            current.status,
            PluginStatus::Running | PluginStatus::Degraded { .. }
        ) || !service(&current.listed.manifest, capability)
            .is_some_and(|spec| current.serving.contains(&spec.service))
        {
            return Err(InstallError::UpstreamUnavailable(self.capability_stopped(
                &current.id,
                capability,
                langs,
            )));
        }
        Ok(current)
    }
}
