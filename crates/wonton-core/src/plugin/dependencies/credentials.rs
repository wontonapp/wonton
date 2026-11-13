use std::collections::BTreeMap;

use super::{
    Connection, InstallError, Languages, Manifest, PluginId, PluginManager, PluginStatus, Provider,
    chosen, connection, declarations, service,
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
    ) -> Result<(), InstallError> {
        let spec = service(&provider.listed.manifest, capability)
            .ok_or_else(|| InstallError::Invalid(self.capability_missing(capability, langs)))?;
        let state = self.dependencies.load()?;
        let existing = connection(&state, consumer, &provider.id, capability);
        let key = spec.credentials.as_ref().map(|_| {
            existing
                .and_then(|c| c.key.clone())
                .unwrap_or_else(crate::secret::random_token)
        });
        let next = Connection {
            consumer: consumer.to_string(),
            provider: provider.id.to_string(),
            capability: capability.to_owned(),
            key: key.clone(),
            ready: existing.is_some_and(|c| c.ready),
            rebuild: existing.is_some_and(|c| c.rebuild),
            problem: Some(self.capability_stopped(&provider.id, capability, langs)),
        };
        self.dependencies.change(|state| {
            state.connections.retain(|c| {
                !(c.consumer == next.consumer
                    && c.provider == next.provider
                    && c.capability == next.capability)
            });
            state.connections.push(next);
        })?;
        if !matches!(
            provider.status,
            PluginStatus::Running | PluginStatus::Degraded { .. }
        ) || !provider.serving.contains(&spec.service)
        {
            return Err(InstallError::UpstreamUnavailable(self.capability_stopped(
                &provider.id,
                capability,
                langs,
            )));
        }
        if let Some(commands) = &spec.credentials {
            let result = self
                .capability_command(
                    provider,
                    capability,
                    commands,
                    true,
                    consumer.as_str(),
                    key.as_deref(),
                    langs,
                )
                .await;
            if let Err(error) = result {
                self.dependencies.change(|state| {
                    if let Some(c) = state.connections.iter_mut().find(|c| {
                        c.consumer == consumer.as_str()
                            && c.provider == provider.id.as_str()
                            && c.capability == capability
                    }) {
                        c.problem = Some(error.to_string());
                    }
                })?;
                return Err(error);
            }
        }
        self.dependencies.change(|state| {
            if let Some(c) = state.connections.iter_mut().find(|c| {
                c.consumer == consumer.as_str()
                    && c.provider == provider.id.as_str()
                    && c.capability == capability
            }) {
                c.problem = None;
                c.ready = true;
            }
        })
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
    ) -> Result<BTreeMap<String, String>, InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let mut vars = BTreeMap::new();
        for capability in declarations(manifest) {
            // Keep confirmed URLs and keys stable while the provider is unavailable; forwarding
            // checks readiness per request, without rebuilding otherwise healthy consumers.
            let connected = chosen(&state, &providers, &manifest.id, capability, None)
                .and_then(|p| connection(&state, &manifest.id, &p.id, capability))
                .filter(|c| c.ready);
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
                    &connection.consumer,
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
                if let Some(connection) = state.connections.iter_mut().find(|connection| {
                    connection.consumer == c.consumer
                        && connection.provider == c.provider
                        && connection.capability == c.capability
                }) {
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
                    &c.consumer,
                    None,
                    langs,
                )
                .await;
            if let Err(error) = result {
                self.dependencies.change(|state| {
                    if let Some(connection) = state.connections.iter_mut().find(|connection| {
                        connection.consumer == c.consumer
                            && connection.provider == c.provider
                            && connection.capability == c.capability
                    }) {
                        connection.problem = Some(error.to_string());
                    }
                })?;
                return Err(error);
            }
        }
        Ok(())
    }
}
