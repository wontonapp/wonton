use wonton_protocol::BotRef;

use super::{
    CapabilityAccessError, CapabilityRoute, InstallError, Languages, PluginId, PluginManager,
    PluginStatus, State, chosen, connection, connection_to, declarations, high,
    selected_binding_bot, service,
};
use crate::plugin::capabilities::AGENT_ACP;

impl PluginManager {
    pub(crate) fn capability_caller_bot(&self, consumer: &PluginId, name: &str) -> Option<BotRef> {
        if self.store.bots().retired(consumer, name).ok()?.is_some() {
            return None;
        }
        let bots = self.store.bots().load(consumer).ok()?;
        bots.bots.contains_key(name).then(|| BotRef {
            plugin_id: consumer.to_string(),
            bot: name.to_owned(),
        })
    }

    pub(super) fn capability_target_bot(
        &self,
        state: &State,
        consumer: &PluginId,
        provider: &PluginId,
        capability: &str,
        bot: Option<&BotRef>,
        langs: &Languages,
    ) -> Result<Option<BotRef>, InstallError> {
        if let Some(name) = selected_binding_bot(state, consumer, capability, bot) {
            return self
                .capability_caller_bot(provider, name)
                .map(Some)
                .ok_or_else(|| self.missing_capability_bot(provider, name, langs));
        }
        if capability != AGENT_ACP.name || bot.is_none() {
            return Ok(None);
        }
        let names = self.stored_bot_names(provider)?;
        match names.as_slice() {
            [name] => Ok(Some(BotRef {
                plugin_id: provider.to_string(),
                bot: name.clone(),
            })),
            _ => Err(InstallError::UpstreamUnavailable(crate::l10n::text(
                langs,
                if names.is_empty() {
                    "gateway-agent-acp-no-bot"
                } else {
                    "gateway-agent-acp-multi-bot"
                },
                &[("provider", provider.as_str().into())],
            ))),
        }
    }

    pub(super) fn missing_capability_bot(
        &self,
        provider: &PluginId,
        bot: &str,
        langs: &Languages,
    ) -> InstallError {
        InstallError::UpstreamUnavailable(crate::l10n::text(
            langs,
            "plugin-capability-bot-missing",
            &[("provider", provider.as_str().into()), ("bot", bot.into())],
        ))
    }

    pub(crate) async fn capability_route(
        &self,
        consumer: &PluginId,
        capability: &str,
        langs: &Languages,
        bot: Option<&BotRef>,
    ) -> Result<CapabilityRoute, CapabilityAccessError> {
        let _changing = self.dependencies.changing.lock().await;
        let bot = bot.filter(|bot| {
            let valid = bot.plugin_id == consumer.as_str()
                && self.capability_caller_bot(consumer, &bot.bot).is_some();
            if !valid {
                tracing::warn!(plugin = %consumer, bot = %bot.bot, "能力调用的 Bot 不在插件记录里，按插件处理");
            }
            valid
        });
        let listed = self
            .plugins
            .read()
            .expect("插件表锁中毒")
            .get(consumer)
            .and_then(|entry| entry.listed.as_ref())
            .filter(|listed| declarations(&listed.manifest).any(|name| name == capability))
            .cloned()
            .ok_or(CapabilityAccessError::Denied)?;
        let providers = self.capability_providers();
        let (provider, target, needs_connection) = self
            .dependencies
            .inspect(|state| {
                let provider =
                    chosen(state, &providers, consumer, capability, bot).ok_or_else(|| {
                        if listed
                            .manifest
                            .requires
                            .iter()
                            .any(|name| name == capability)
                        {
                            CapabilityAccessError::Denied
                        } else {
                            CapabilityAccessError::Unavailable(
                                self.capability_missing(capability, langs),
                            )
                        }
                    })?;
                let target = self
                    .capability_target_bot(state, consumer, &provider.id, capability, bot, langs)
                    .map_err(unavailable)?;
                let target_name = target.as_ref().map(|bot| bot.bot.as_str());
                let connected = if bot.is_some() {
                    connection_to(state, consumer, &provider.id, capability, bot, target_name)
                } else {
                    connection(state, consumer, &provider.id, capability, None)
                };
                if let Some(connected) = connected {
                    if let Some(problem) = &connected.problem {
                        return Err(CapabilityAccessError::Unavailable(problem.clone()));
                    }
                    if !connected.ready {
                        return Err(CapabilityAccessError::Unavailable(self.capability_stopped(
                            &provider.id,
                            capability,
                            langs,
                        )));
                    }
                } else if bot.is_none()
                    || high(&provider.listed.manifest, capability)
                    || connection_to(state, consumer, &provider.id, capability, None, target_name)
                        .is_none()
                {
                    return Err(CapabilityAccessError::Denied);
                }
                Ok((provider, target, connected.is_none()))
            })
            .map_err(unavailable)??;
        if needs_connection {
            self.connect_capability(consumer, provider, capability, langs, bot)
                .await
                .map_err(unavailable)?;
        }
        let spec =
            service(&provider.listed.manifest, capability).ok_or(CapabilityAccessError::Denied)?;
        if !matches!(
            provider.status,
            PluginStatus::Running | PluginStatus::Degraded { .. }
        ) || !provider.serving.contains(&spec.service)
        {
            return Err(CapabilityAccessError::Unavailable(self.capability_stopped(
                &provider.id,
                capability,
                langs,
            )));
        }
        drop(_changing);
        self.finish_capability_route(consumer, capability, provider, spec, target, langs)
            .await
    }

    async fn finish_capability_route(
        &self,
        _consumer: &PluginId,
        capability: &str,
        provider: &super::Provider,
        spec: &super::ProviderService,
        target: Option<BotRef>,
        langs: &Languages,
    ) -> Result<CapabilityRoute, CapabilityAccessError> {
        #[cfg(test)]
        if let Some(endpoint) = self
            .dependencies
            .routes
            .read()
            .expect("测试能力路由锁中毒")
            .get(&(_consumer.to_string(), capability.to_owned()))
            .cloned()
        {
            return Ok(CapabilityRoute {
                provider: provider.id.clone(),
                endpoint,
                bot: target,
            });
        }
        let endpoint = self
            .capability_endpoint(&provider.listed.manifest, &spec.service, spec.port)
            .await
            .map_err(unavailable)?
            .ok_or_else(|| {
                CapabilityAccessError::Unavailable(self.capability_stopped(
                    &provider.id,
                    capability,
                    langs,
                ))
            })?;
        Ok(CapabilityRoute {
            provider: provider.id.clone(),
            endpoint,
            bot: target,
        })
    }
}

fn unavailable(error: InstallError) -> CapabilityAccessError {
    CapabilityAccessError::Unavailable(error.to_string())
}
