use super::store::HandoffConsent;
use super::{
    CapabilityAccessError, CapabilityRoute, PluginId, PluginManager, PluginStatus, declarations,
    service,
};
use crate::plugin::capabilities::AGENT_ACP;
use crate::plugin::languages::Languages;
use wonton_protocol::BotRef;

impl PluginManager {
    pub(crate) fn declares_agent_acp(&self, consumer: &PluginId) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(consumer)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| declarations(&listed.manifest).any(|name| name == AGENT_ACP.name))
    }

    /// 指定交接目标不会修改默认提供方；这次连接由目标归属人的卡片授权。
    pub(crate) async fn handoff_route(
        &self,
        consumer: &PluginId,
        target: &BotRef,
        langs: &Languages,
    ) -> Result<CapabilityRoute, CapabilityAccessError> {
        if !self.declares_agent_acp(consumer) {
            return Err(CapabilityAccessError::Denied);
        }
        let provider =
            PluginId::parse(&target.plugin_id).map_err(|_| CapabilityAccessError::Denied)?;
        if self
            .capability_caller_bot(&provider, &target.bot)
            .map_err(|error| CapabilityAccessError::Unavailable(error.to_string()))?
            .is_none()
        {
            return Err(CapabilityAccessError::Unavailable(
                self.missing_capability_bot(&provider, &target.bot, langs)
                    .to_string(),
            ));
        }
        let (listed, spec) = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            let entry = plugins
                .get(&provider)
                .ok_or(CapabilityAccessError::Denied)?;
            let listed = entry.listed.as_ref().ok_or(CapabilityAccessError::Denied)?;
            let spec =
                service(&listed.manifest, AGENT_ACP.name).ok_or(CapabilityAccessError::Denied)?;
            if !matches!(
                entry.status,
                PluginStatus::Running | PluginStatus::Degraded { .. }
            ) || !entry.serving.contains(&spec.service)
            {
                return Err(CapabilityAccessError::Unavailable(self.capability_stopped(
                    &provider,
                    AGENT_ACP.name,
                    langs,
                )));
            }
            (listed.clone(), (spec.service.clone(), spec.port))
        };
        #[cfg(test)]
        if let Some(endpoint) = self
            .dependencies
            .routes
            .read()
            .expect("测试能力路由锁中毒")
            .get(&(consumer.to_string(), AGENT_ACP.name.to_owned()))
            .cloned()
        {
            return Ok(CapabilityRoute {
                provider,
                endpoint,
                bot: Some(target.clone()),
            });
        }
        let endpoint = self
            .capability_endpoint(&listed.manifest, &spec.0, spec.1)
            .await
            .map_err(|error| CapabilityAccessError::Unavailable(error.to_string()))?
            .ok_or_else(|| {
                CapabilityAccessError::Unavailable(self.capability_stopped(
                    &provider,
                    AGENT_ACP.name,
                    langs,
                ))
            })?;
        Ok(CapabilityRoute {
            provider,
            endpoint,
            bot: Some(target.clone()),
        })
    }

    pub(crate) fn handoff_confirmed(
        &self,
        from: &BotRef,
        target: &BotRef,
    ) -> Result<bool, super::InstallError> {
        self.dependencies.inspect(|state| {
            state
                .handoff_consents
                .iter()
                .any(|consent| consent.from == *from && consent.target == *target)
        })
    }

    pub(crate) fn confirm_handoff(
        &self,
        from: &BotRef,
        target: &BotRef,
    ) -> Result<(), super::InstallError> {
        self.dependencies.change(|state| {
            if !state
                .handoff_consents
                .iter()
                .any(|consent| consent.from == *from && consent.target == *target)
            {
                state.handoff_consents.push(HandoffConsent {
                    from: from.clone(),
                    target: target.clone(),
                });
            }
        })
    }
}
