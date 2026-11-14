use wonton_protocol::BotRef;

use super::{
    InstallError, Languages, Manifest, PluginManager, chosen, connection_to, declarations, high,
};

impl PluginManager {
    pub(in crate::plugin) async fn prepare_bot_dependencies(
        &self,
        manifest: &Manifest,
        bot: &str,
        langs: &Languages,
        force_provider: Option<&super::PluginId>,
    ) -> Result<(), InstallError> {
        if self.store.bots().retired(&manifest.id, bot)?.is_some() {
            return Ok(());
        }
        let caller = BotRef {
            plugin_id: manifest.id.to_string(),
            bot: bot.to_owned(),
        };
        let providers = self.capability_providers();
        let state = self.dependencies.load()?;
        for capability in declarations(manifest) {
            let Some(provider) =
                chosen(&state, &providers, &manifest.id, capability, Some(&caller))
            else {
                continue;
            };
            let target = match self.capability_target_bot(
                &state,
                &manifest.id,
                &provider.id,
                capability,
                Some(&caller),
                langs,
            ) {
                Ok(target) => target,
                Err(InstallError::UpstreamUnavailable(_)) => continue,
                Err(error) => return Err(error),
            };
            let target_name = target.as_ref().map(|bot| bot.bot.as_str());
            let existing = connection_to(
                &state,
                &manifest.id,
                &provider.id,
                capability,
                Some(&caller),
                target_name,
            );
            if existing.is_some_and(|connection| connection.ready && connection.problem.is_none())
                && force_provider != Some(&provider.id)
            {
                continue;
            }
            if existing.is_none()
                && (high(&provider.listed.manifest, capability)
                    || connection_to(
                        &state,
                        &manifest.id,
                        &provider.id,
                        capability,
                        None,
                        target_name,
                    )
                    .is_none())
            {
                continue;
            }
            if let Err(error) = self
                .connect_capability(&manifest.id, provider, capability, langs, Some(&caller))
                .await
                && (manifest.requires.contains(capability)
                    || matches!(error, InstallError::Storage(_)))
            {
                return Err(error);
            }
        }
        Ok(())
    }

    /// 主人确定的连接选择；Bot 参数来自核心记录，不增加跨端协议类型。
    pub async fn set_capability_binding(
        &self,
        consumer: &super::PluginId,
        consumer_bot: Option<&str>,
        choice: wonton_protocol::ConsumerCapabilityChoice,
        provider_bot: Option<&str>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let _changing = self.dependencies.changing.lock().await;
        let record = self
            .store
            .load(consumer)?
            .ok_or_else(|| InstallError::not_installed(langs, consumer))?;
        let package = crate::plugin::upgrade::installed_package(&record, langs)?;
        if !declarations(&package.manifest).any(|name| name == &choice.capability)
            || (provider_bot.is_some() && choice.provider.is_none())
            || choice.provider.as_deref() == Some(consumer.as_str())
        {
            return Err(InstallError::Invalid(self.capability_missing_other(
                &package.manifest,
                &choice.capability,
                langs,
            )));
        }
        let caller = consumer_bot
            .map(|name| {
                self.capability_caller_bot(consumer, name)?
                    .ok_or_else(|| self.missing_capability_bot(consumer, name, langs))
            })
            .transpose()?;
        let mut next = self.dependencies.load()?;
        next.select(
            consumer.as_str(),
            consumer_bot,
            &choice.capability,
            choice.provider.as_deref(),
            provider_bot,
        );
        let providers = self.capability_providers();
        let selected = chosen(
            &next,
            &providers,
            consumer,
            &choice.capability,
            caller.as_ref(),
        );
        if (selected.is_none() && package.manifest.requires.contains(&choice.capability))
            || choice
                .provider
                .as_deref()
                .is_some_and(|id| selected.is_none_or(|selected| selected.id.as_str() != id))
        {
            return Err(InstallError::Invalid(
                self.capability_missing(&choice.capability, langs),
            ));
        }
        if let Some(provider) = selected {
            self.capability_target_bot(
                &next,
                consumer,
                &provider.id,
                &choice.capability,
                caller.as_ref(),
                langs,
            )?;
        }
        self.dependencies.change(|state| {
            state.selected = next.selected;
        })?;
        let connected = match selected {
            Some(provider) => {
                self.connect_capability(
                    consumer,
                    provider,
                    &choice.capability,
                    langs,
                    caller.as_ref(),
                )
                .await
            }
            None => Ok(()),
        };
        let rebuilt = match consumer_bot {
            Some(bot) => self.rebuild_capability_bot(consumer, bot, langs).await,
            None => self.rebuild_capability_consumer(consumer, langs).await,
        };
        self.changed();
        match (connected, rebuilt) {
            (Err(error), Err(cleanup)) => Err(error.with_note(langs, &cleanup.to_string())),
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => Ok(()),
        }
    }

    #[cfg(test)]
    pub(crate) fn capability_bot_route_for_test(
        &self,
        consumer: &BotRef,
        capability: &str,
        provider: &super::PluginId,
        provider_bot: Option<&str>,
        endpoint: crate::plugin::upstreams::Endpoint,
    ) {
        let consumer_id = super::PluginId::parse(&consumer.plugin_id).unwrap();
        let langs = self.owner_languages();
        self.dependencies
            .change(|state| {
                state.select(
                    &consumer.plugin_id,
                    Some(&consumer.bot),
                    capability,
                    Some(provider.as_str()),
                    provider_bot,
                );
                let target = self
                    .capability_target_bot(
                        state,
                        &consumer_id,
                        provider,
                        capability,
                        Some(consumer),
                        &langs,
                    )
                    .unwrap();
                let next = super::Connection {
                    consumer: consumer.plugin_id.clone(),
                    consumer_bot: Some(consumer.bot.clone()),
                    provider: provider.to_string(),
                    provider_bot: target.map(|bot| bot.bot),
                    capability: capability.to_owned(),
                    ready: true,
                    ..Default::default()
                };
                state
                    .connections
                    .retain(|connection| !connection.same_pair(&next));
                state.connections.push(next);
            })
            .unwrap();
        self.dependencies.routes.write().unwrap().insert(
            (consumer.plugin_id.clone(), capability.to_owned()),
            endpoint,
        );
    }
}
