use std::collections::{BTreeMap, BTreeSet};

use super::binding::{Binding, consumer_key};
use super::{
    Connection, InstallError, Languages, Manifest, PluginId, PluginManager, PluginStatus, State,
    chosen, connection, declarations, high, selected_binding_bot, service,
};

pub(in crate::plugin) struct BotRemovalRecovery(Vec<ConsumerRecovery>);

struct ConsumerRecovery {
    consumer: String,
    bot: Option<String>,
    manifest: Manifest,
    vars: BTreeMap<String, String>,
    was_connected: bool,
}

pub(in crate::plugin) struct BotRemovalReplacements(Vec<BotRemovalReplacement>);

struct BotRemovalReplacement {
    consumer: String,
    bot: Option<String>,
    capability: String,
    binding: Binding,
}

impl BotRemovalReplacement {
    fn preserves(&self, connection: &Connection) -> bool {
        self.consumer == connection.consumer
            && self.bot == connection.consumer_bot
            && self.capability == connection.capability
            && self.binding.provider() == Some(connection.provider.as_str())
            && self.binding.bot() == connection.provider_bot.as_deref()
    }
}

impl PluginManager {
    pub(in crate::plugin) fn check_capability_bot_removal(
        &self,
        plugin: &PluginId,
        bot: &str,
        langs: &Languages,
    ) -> Result<BotRemovalReplacements, InstallError> {
        let Some(record) = self.store.load(plugin)? else {
            return Ok(BotRemovalReplacements(Vec::new()));
        };
        let bots = self.store.bots().load(plugin)?;
        if !bots.bots.contains_key(bot) {
            return Ok(BotRemovalReplacements(Vec::new()));
        }
        for other in bots.bots.keys().filter(|name| name.as_str() != bot) {
            if self.store.bots().retired(plugin, other)?.is_none() {
                return Ok(BotRemovalReplacements(Vec::new()));
            }
        }
        let package = super::super::upgrade::installed_package(&record, langs)?;
        let mut replacement = package.manifest.clone();
        let bot_services = super::super::bot_services::bot_services(&package.manifest);
        if let Some(provides) = replacement.provides.as_mut() {
            provides.capabilities.retain(|capability, spec| {
                capability != crate::plugin::capabilities::AGENT_ACP.name
                    && package.manifest.bots.is_some()
                    && !bot_services.contains(spec.service.as_str())
            });
        }
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let mut replacements = Vec::new();
        let mut blocked = Vec::new();
        for consumer in self.capability_consumers() {
            match self.required_bot_replacements(
                &state,
                &providers,
                &consumer.manifest,
                (plugin, bot),
                &replacement,
            )? {
                Some(choices) => replacements.extend(choices),
                None => blocked.push(wonton_protocol::CapabilityProviderRef {
                    id: consumer.manifest.id.to_string(),
                    title: langs.pick_owned(&consumer.manifest.title),
                    high_privilege: false,
                }),
            }
        }
        if blocked.is_empty() {
            return Ok(BotRemovalReplacements(replacements));
        }
        let titles: Vec<_> = blocked.iter().map(|p| p.title.as_str()).collect();
        Err(InstallError::InUse {
            message: crate::l10n::text(
                langs,
                "plugin-capability-in-use",
                &[("plugins", crate::l10n::list(langs, &titles).into())],
            ),
            plugins: blocked,
        })
    }

    fn required_bot_replacements(
        &self,
        state: &State,
        providers: &[super::Provider],
        consumer: &Manifest,
        retiring: (&PluginId, &str),
        replacement: &Manifest,
    ) -> Result<Option<Vec<BotRemovalReplacement>>, InstallError> {
        let bots = self.active_capability_bots(&consumer.id)?;
        let mut choices = Vec::new();
        for capability in consumer.requires.iter().filter(|c| super::generic(c)) {
            for caller in std::iter::once(None).chain(bots.iter().map(Some)) {
                if !chosen(state, providers, &consumer.id, capability, caller)
                    .is_some_and(|provider| provider.id == *retiring.0)
                    || (service(replacement, capability).is_some()
                        && selected_binding_bot(state, &consumer.id, capability, caller)
                            != Some(retiring.1))
                {
                    continue;
                }
                let candidates = state.connections.iter().filter(|c| {
                    c.consumer == consumer.id.as_str()
                        && c.capability == *capability
                        && (c.consumer_bot.as_deref() == caller.map(|bot| bot.bot.as_str())
                            || c.consumer_bot.is_none())
                });
                let Some(binding) = self.confirmed_bot_replacement(
                    candidates,
                    providers,
                    retiring,
                    replacement,
                    caller.is_some(),
                )?
                else {
                    return Ok(None);
                };
                choices.push(BotRemovalReplacement {
                    consumer: consumer.id.to_string(),
                    bot: caller.map(|bot| bot.bot.clone()),
                    capability: capability.clone(),
                    binding,
                });
            }
        }
        Ok(Some(choices))
    }

    fn confirmed_bot_replacement<'a>(
        &self,
        candidates: impl Iterator<Item = &'a Connection>,
        providers: &[super::Provider],
        retiring: (&PluginId, &str),
        replacement: &Manifest,
        calling_bot: bool,
    ) -> Result<Option<Binding>, InstallError> {
        for c in candidates.filter(|c| c.ready && c.problem.is_none()) {
            let Some(provider) = providers.iter().find(|p| p.id.as_str() == c.provider) else {
                continue;
            };
            if provider.id.as_str() == c.consumer
                || (provider.id == *retiring.0 && service(replacement, &c.capability).is_none())
                || !matches!(
                    provider.status,
                    PluginStatus::Running | PluginStatus::Degraded { .. }
                )
                || !service(&provider.listed.manifest, &c.capability)
                    .is_some_and(|spec| provider.serving.contains(&spec.service))
                || (calling_bot
                    && c.consumer_bot.is_none()
                    && high(&provider.listed.manifest, &c.capability))
            {
                continue;
            }
            let binding = if let Some(bot) = &c.provider_bot {
                if (provider.id == *retiring.0 && bot == retiring.1)
                    || self.capability_caller_bot(&provider.id, bot)?.is_none()
                {
                    continue;
                }
                Binding::Bot(wonton_protocol::BotRef {
                    plugin_id: c.provider.clone(),
                    bot: bot.clone(),
                })
            } else {
                if c.capability == crate::plugin::capabilities::AGENT_ACP.name
                    && (calling_bot || self.active_capability_bots(&provider.id)?.is_empty())
                {
                    continue;
                }
                Binding::Plugin(c.provider.clone())
            };
            return Ok(Some(binding));
        }
        Ok(None)
    }

    /// 调用方持有 dependencies.changing 和 bot_allocation。
    pub(in crate::plugin) async fn forget_capability_bot(
        &self,
        plugin: &PluginId,
        bot: &str,
        replacements: BotRemovalReplacements,
        langs: &Languages,
    ) -> Result<BotRemovalRecovery, InstallError> {
        let before = self.dependencies.load()?;
        let mut affected = self.capability_bot_removal_scopes(&before, plugin, bot)?;
        for replacement in &replacements.0 {
            affected.insert((replacement.consumer.clone(), replacement.bot.clone()));
        }
        let mut snapshots = Vec::with_capacity(affected.len());
        for (consumer, consumer_bot) in affected {
            let id =
                PluginId::parse(&consumer).map_err(|e| InstallError::Storage(e.to_string()))?;
            let Some(record) = self.store.load(&id)? else {
                continue;
            };
            let manifest = super::super::upgrade::installed_package(&record, langs)?.manifest;
            let vars = self.dependency_vars(&manifest, consumer_bot.as_deref())?;
            let was_connected = before.connections.iter().any(|c| {
                c.consumer == manifest.id.as_str()
                    && c.consumer_bot.as_deref() == consumer_bot.as_deref()
                    && c.provider == plugin.as_str()
                    && c.provider_bot.as_deref() == Some(bot)
                    && c.ready
            });
            snapshots.push(ConsumerRecovery {
                consumer,
                bot: consumer_bot,
                manifest,
                vars,
                was_connected,
            });
        }
        let providers = self.capability_providers();
        let invalidated = invalidated_bot_connections(&before, &providers, plugin, bot);
        let removing = |c: &Connection| {
            (c.consumer == plugin.as_str() && c.consumer_bot.as_deref() == Some(bot))
                || (c.provider == plugin.as_str() && c.provider_bot.as_deref() == Some(bot))
                || (invalidated.iter().any(|(consumer, bot, capability)| {
                    c.consumer == *consumer && c.consumer_bot == *bot && c.capability == *capability
                }) && !replacements
                    .0
                    .iter()
                    .any(|replacement| replacement.preserves(c)))
        };
        self.dependencies.change(|state| {
            for c in state.connections.iter_mut().filter(|c| removing(c)) {
                c.ready = false;
                c.problem = Some(self.capability_stopped(plugin, &c.capability, langs));
            }
        })?;
        for c in before.connections.iter().filter(|c| removing(c)) {
            self.revoke_capability_bot_connection(c, &providers, langs)
                .await?;
        }
        self.dependencies.change(|state| {
            state.connections.retain(|c| !removing(c));
            state.selected.remove(&consumer_key(plugin.as_str(), bot));
            for choices in state.selected.values_mut() {
                choices.retain(|_, binding| !targets_bot(binding, plugin, bot));
            }
            state.selected.retain(|_, choices| !choices.is_empty());
            for replacement in &replacements.0 {
                state.select(
                    &replacement.consumer,
                    replacement.bot.as_deref(),
                    &replacement.capability,
                    replacement.binding.provider(),
                    replacement.binding.bot(),
                );
            }
            state.handoff_consents.retain(|consent| {
                !(consent.from.plugin_id == plugin.as_str() && consent.from.bot == bot)
                    && !(consent.target.plugin_id == plugin.as_str() && consent.target.bot == bot)
            });
        })?;
        Ok(BotRemovalRecovery(snapshots))
    }

    /// 调用方持有 dependencies.changing，不持有 bot_allocation。
    pub(in crate::plugin) async fn recover_capability_bot_consumers(
        &self,
        recovery: BotRemovalRecovery,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let mut recovered = BTreeSet::new();
        let mut failure: Option<InstallError> = None;
        for ConsumerRecovery {
            consumer,
            bot: consumer_bot,
            manifest,
            vars,
            was_connected,
        } in recovery.0
        {
            let mut prepared = Ok(());
            if recovered.insert(consumer) {
                prepared = self.recover_ordinary_bot_fallbacks(&manifest, langs).await;
            }
            if let Some(consumer_bot) = &consumer_bot {
                prepared = combine(
                    prepared,
                    self.prepare_bot_dependencies(&manifest, consumer_bot, langs, None)
                        .await,
                    langs,
                );
            }
            let changed =
                vars != self.dependency_vars(&manifest, consumer_bot.as_deref())? || was_connected;
            let rebuilt = if changed {
                match consumer_bot.as_deref() {
                    Some(consumer_bot) => {
                        self.rebuild_capability_bot(&manifest.id, consumer_bot, langs)
                            .await
                    }
                    None => self.rebuild_capability_consumer(&manifest.id, langs).await,
                }
            } else {
                Ok(())
            };
            if let Err(error) = combine(prepared, rebuilt, langs) {
                failure = Some(match failure {
                    Some(previous) => error.with_note(langs, &previous.to_string()),
                    None => error,
                });
            }
        }
        self.changed();
        failure.map_or(Ok(()), Err)
    }

    fn capability_bot_removal_scopes(
        &self,
        state: &State,
        plugin: &PluginId,
        bot: &str,
    ) -> Result<BTreeSet<(String, Option<String>)>, InstallError> {
        let mut affected = BTreeSet::new();
        for c in &state.connections {
            if c.provider == plugin.as_str() && c.provider_bot.as_deref() == Some(bot) {
                affected.insert((c.consumer.clone(), c.consumer_bot.clone()));
            }
        }
        for (scope, choices) in &state.selected {
            if choices
                .values()
                .any(|binding| targets_bot(binding, plugin, bot))
            {
                let (consumer, bot) = scope
                    .split_once('/')
                    .map_or((scope.as_str(), None), |(id, bot)| {
                        (id, Some(bot.to_owned()))
                    });
                affected.insert((consumer.to_owned(), bot));
            }
        }
        let inherited: Vec<_> = affected
            .iter()
            .filter(|(_, bot)| bot.is_none())
            .cloned()
            .collect();
        for (consumer, _) in inherited {
            let id =
                PluginId::parse(&consumer).map_err(|e| InstallError::Storage(e.to_string()))?;
            for name in self.store.bots().load(&id)?.bots.keys() {
                if self.store.bots().retired(&id, name)?.is_none() {
                    affected.insert((consumer.clone(), Some(name.clone())));
                }
            }
        }
        affected.remove(&(plugin.to_string(), Some(bot.to_owned())));
        Ok(affected)
    }

    async fn revoke_capability_bot_connection(
        &self,
        c: &Connection,
        providers: &[super::Provider],
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let Some(provider) = providers.iter().find(|p| p.id.as_str() == c.provider) else {
            return Ok(());
        };
        let Some(commands) = service(&provider.listed.manifest, &c.capability)
            .and_then(|spec| spec.credentials.as_ref())
        else {
            return Ok(());
        };
        if let Err(error) = self
            .capability_command(
                provider,
                &c.capability,
                commands,
                false,
                &c.credential_name(),
                None,
                langs,
            )
            .await
        {
            self.dependencies.change(|state| {
                if let Some(stored) = state
                    .connections
                    .iter_mut()
                    .find(|stored| stored.same_pair(c))
                {
                    stored.problem = Some(error.to_string());
                }
            })?;
            return Err(error);
        }
        Ok(())
    }

    async fn recover_ordinary_bot_fallbacks(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let providers = self.capability_providers();
        for capability in declarations(manifest) {
            let state = self.dependencies.load()?;
            let Some(provider) = chosen(&state, &providers, &manifest.id, capability, None) else {
                continue;
            };
            if high(&provider.listed.manifest, capability)
                || connection(&state, &manifest.id, &provider.id, capability, None)
                    .is_some_and(|c| c.ready && c.problem.is_none())
            {
                continue;
            }
            if let Err(error) = self
                .connect_capability(&manifest.id, provider, capability, langs, None)
                .await
                && (manifest.requires.contains(capability)
                    || matches!(error, InstallError::Storage(_)))
            {
                return Err(error);
            }
        }
        Ok(())
    }
}

fn targets_bot(binding: &Binding, plugin: &PluginId, bot: &str) -> bool {
    binding.provider() == Some(plugin.as_str()) && binding.bot() == Some(bot)
}

fn invalidated_bot_connections(
    state: &State,
    providers: &[super::Provider],
    plugin: &PluginId,
    bot: &str,
) -> BTreeSet<(String, Option<String>, String)> {
    state
        .connections
        .iter()
        .filter(|c| c.provider == plugin.as_str() && c.provider_bot.as_deref() == Some(bot))
        .filter(|c| {
            providers
                .iter()
                .any(|p| p.id.as_str() == c.provider && high(&p.listed.manifest, &c.capability))
        })
        .map(|c| {
            (
                c.consumer.clone(),
                c.consumer_bot.clone(),
                c.capability.clone(),
            )
        })
        .collect()
}

fn combine(
    first: Result<(), InstallError>,
    second: Result<(), InstallError>,
    langs: &Languages,
) -> Result<(), InstallError> {
    match (first, second) {
        (Err(error), Err(cleanup)) => Err(error.with_note(langs, &cleanup.to_string())),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}
