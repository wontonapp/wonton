use std::collections::BTreeMap;
use wonton_protocol::BotRef;

use super::{
    Connection, InstallError, Languages, Manifest, PluginId, PluginManager, PluginStatus, Provider,
    chosen, connection_to, declarations, service,
};
use crate::plugin::capabilities;
use crate::plugin::provides::{CredentialCommands, NAME_PLACEHOLDER};

impl PluginManager {
    pub(super) async fn connect_capability(
        &self,
        consumer: &PluginId,
        provider: &Provider,
        capability: &str,
        langs: &Languages,
        bot: Option<&BotRef>,
    ) -> Result<(), InstallError> {
        let spec = service(&provider.listed.manifest, capability)
            .ok_or_else(|| InstallError::Invalid(self.capability_missing(capability, langs)))?;
        let state = self.dependencies.load()?;
        let target =
            self.capability_target_bot(&state, consumer, &provider.id, capability, bot, langs)?;
        let target_name = target.as_ref().map(|bot| bot.bot.as_str());
        let existing = connection_to(&state, consumer, &provider.id, capability, bot, target_name);
        let key = spec.credentials.as_ref().map(|_| {
            existing
                .and_then(|c| c.key.clone())
                .unwrap_or_else(crate::secret::random_token)
        });
        let next = Connection {
            consumer: consumer.to_string(),
            consumer_bot: bot.map(|bot| bot.bot.clone()),
            provider: provider.id.to_string(),
            provider_bot: target.as_ref().map(|bot| bot.bot.clone()),
            capability: capability.to_owned(),
            key: key.clone(),
            ready: existing.is_some_and(|c| c.ready),
            rebuild: existing.is_some_and(|c| c.rebuild),
            problem: Some(self.capability_stopped(&provider.id, capability, langs)),
        };
        self.dependencies.change(|state| {
            state.connections.retain(|c| !c.same_pair(&next));
            state.connections.push(next);
        })?;
        let result = if !matches!(
            provider.status,
            PluginStatus::Running | PluginStatus::Degraded { .. }
        ) || !provider.serving.contains(&spec.service)
        {
            Err(InstallError::UpstreamUnavailable(self.capability_stopped(
                &provider.id,
                capability,
                langs,
            )))
        } else if let Some(commands) = &spec.credentials {
            let name = super::binding::credential_name(
                consumer.as_str(),
                bot.map(|bot| bot.bot.as_str()),
                target_name,
            );
            self.capability_command(
                provider,
                capability,
                commands,
                true,
                &name,
                key.as_deref(),
                langs,
            )
            .await
        } else {
            Ok(())
        };
        self.dependencies.change(|state| {
            if let Some(c) = state.connections.iter_mut().find(|c| {
                c.consumer == consumer.as_str()
                    && c.consumer_bot.as_deref() == bot.map(|bot| bot.bot.as_str())
                    && c.provider == provider.id.as_str()
                    && c.provider_bot.as_deref() == target_name
                    && c.capability == capability
            }) {
                c.problem = result.as_ref().err().map(ToString::to_string);
                c.ready |= result.is_ok();
            }
        })?;
        result
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn capability_command(
        &self,
        provider: &Provider,
        capability: &str,
        commands: &CredentialCommands,
        issue: bool,
        name: &str,
        key: Option<&str>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let spec = service(&provider.listed.manifest, capability)
            .ok_or_else(|| InstallError::Invalid(self.capability_missing(capability, langs)))?;
        let container = self.cfg.instance.container(
            &provider.id.slug(),
            commands.service.as_deref().unwrap_or(&spec.service),
        );
        let argv: Vec<_> = if issue {
            &commands.issue
        } else {
            &commands.revoke
        }
        .iter()
        .map(|arg| arg.replace(NAME_PLACEHOLDER, name))
        .collect();
        let input = key.map(|value| format!("{value}\n"));
        let result = self
            .engine
            .exec(
                &container,
                &argv,
                input.as_deref().map(str::as_bytes),
                super::super::models::COMMAND_TIMEOUT,
            )
            .await?;
        if result.exit_code == 0 {
            return Ok(());
        }
        Err(InstallError::UpstreamUnavailable(crate::l10n::text(
            langs,
            "plugin-capability-credential-failed",
            &[
                ("provider", provider.id.as_str().into()),
                ("capability", capability.into()),
                ("code", result.exit_code.to_string().into()),
                ("detail", result.stderr.into()),
            ],
        )))
    }

    pub(in crate::plugin) fn dependency_vars(
        &self,
        manifest: &Manifest,
        bot: Option<&str>,
    ) -> Result<BTreeMap<String, String>, InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let mut vars = BTreeMap::new();
        let bot = bot.map(|bot| BotRef {
            plugin_id: manifest.id.to_string(),
            bot: bot.to_owned(),
        });
        for capability in declarations(manifest) {
            // Keep confirmed URLs and keys stable while the provider is unavailable; forwarding
            // checks readiness per request, without rebuilding otherwise healthy consumers.
            let provider = chosen(&state, &providers, &manifest.id, capability, bot.as_ref());
            let target = provider
                .map(|provider| {
                    self.capability_target_bot(
                        &state,
                        &manifest.id,
                        &provider.id,
                        capability,
                        bot.as_ref(),
                        &self.owner_languages(),
                    )
                })
                .transpose();
            let connected = match target {
                Ok(target) => provider
                    .and_then(|p| {
                        connection_to(
                            &state,
                            &manifest.id,
                            &p.id,
                            capability,
                            bot.as_ref(),
                            target
                                .as_ref()
                                .and_then(Option::as_ref)
                                .map(|bot| bot.bot.as_str()),
                        )
                    })
                    .filter(|c| c.ready),
                Err(InstallError::UpstreamUnavailable(_)) => None,
                Err(error) => return Err(error),
            };
            vars.insert(
                capabilities::var_url(capability),
                connected.map_or_else(String::new, |_| {
                    format!(
                        "http://{}:{}",
                        capabilities::alias(capability),
                        self.cfg.gateway_port
                    )
                }),
            );
            if capabilities::lookup(capability)
                .is_none_or(|spec| spec.credentials != capabilities::Credentials::None)
            {
                vars.insert(
                    capabilities::var_key(capability),
                    connected.and_then(|c| c.key.clone()).unwrap_or_default(),
                );
            }
        }
        Ok(vars)
    }

    pub(in crate::plugin) async fn revoke_removed_provider_keys(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let providers = self.capability_providers();
        let Some(provider) = providers.iter().find(|p| p.id == manifest.id) else {
            return Ok(());
        };
        let state = self.dependencies.load()?;
        for connection in state.connections.iter().filter(|c| {
            c.provider == manifest.id.as_str() && service(manifest, &c.capability).is_none()
        }) {
            if let Some(commands) = service(&provider.listed.manifest, &connection.capability)
                .and_then(|s| s.credentials.as_ref())
            {
                self.capability_command(
                    provider,
                    &connection.capability,
                    commands,
                    false,
                    &connection.credential_name(),
                    None,
                    langs,
                )
                .await?;
            }
        }
        Ok(())
    }

    pub(in crate::plugin) async fn revoke_dependency_keys(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        for c in state
            .connections
            .iter()
            .filter(|c| c.consumer == id.as_str() || c.provider == id.as_str())
        {
            let Some(provider) = providers.iter().find(|p| p.id.as_str() == c.provider) else {
                continue;
            };
            let Some(commands) = service(&provider.listed.manifest, &c.capability)
                .and_then(|s| s.credentials.as_ref())
            else {
                continue;
            };
            self.dependencies.change(|state| {
                if let Some(connection) = state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.same_pair(c))
                {
                    connection.problem =
                        Some(self.capability_stopped(&provider.id, &c.capability, langs));
                }
            })?;
            let result = self
                .capability_command(
                    provider,
                    &c.capability,
                    commands,
                    false,
                    &c.credential_name(),
                    None,
                    langs,
                )
                .await;
            if let Err(error) = result {
                self.dependencies.change(|state| {
                    if let Some(connection) = state
                        .connections
                        .iter_mut()
                        .find(|connection| connection.same_pair(c))
                    {
                        connection.problem = Some(error.to_string());
                    }
                })?;
                return Err(error);
            }
        }
        Ok(())
    }
}
