use std::collections::BTreeMap;

use super::{Connection, InstallError, Languages, Manifest, PluginManager, State, service};
use crate::plugin::package::PluginPackage;
use crate::plugin::package_files::sha256_hex;

pub(in crate::plugin) struct CapabilityStateSnapshot(State);

impl PluginManager {
    pub(crate) fn dependency_plan(&self, package: &PluginPackage) -> Result<String, InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let consumers = self.capability_consumers();
        let provider_versions: Vec<_> = providers
            .iter()
            .map(|provider| (provider.id.as_str(), provider.listed.revision.as_str()))
            .collect();
        let consumer_versions: Vec<_> = consumers
            .iter()
            .map(|consumer| (consumer.manifest.id.as_str(), consumer.revision.as_str()))
            .collect();
        let mut connections: Vec<_> = state
            .connections
            .iter()
            .map(|connection| {
                (
                    connection.consumer.as_str(),
                    connection.provider.as_str(),
                    connection.capability.as_str(),
                )
            })
            .collect();
        connections.sort_unstable();
        let binding = serde_json::to_vec(&(
            package.digest(),
            provider_versions,
            consumer_versions,
            &state.defaults,
            &state.selected,
            connections,
        ))
        .map_err(|error| InstallError::Storage(error.to_string()))?;
        Ok(sha256_hex(&binding))
    }

    pub(in crate::plugin) fn snapshot_capability_state(
        &self,
    ) -> Result<CapabilityStateSnapshot, InstallError> {
        self.dependencies.load().map(CapabilityStateSnapshot)
    }

    pub(in crate::plugin) async fn restore_capability_state(
        &self,
        snapshot: CapabilityStateSnapshot,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let now = self.dependencies.load()?;
        let providers = self.capability_providers();
        let mut failure: Option<String> = None;
        for connection in now.connections.iter().filter(|connection| {
            !snapshot
                .0
                .connections
                .iter()
                .any(|before| same_connection(before, connection))
        }) {
            let Some(provider) = providers
                .iter()
                .find(|provider| provider.id.as_str() == connection.provider)
            else {
                continue;
            };
            let Some(commands) = service(&provider.listed.manifest, &connection.capability)
                .and_then(|spec| spec.credentials.as_ref())
            else {
                continue;
            };
            if let Err(error) = self
                .capability_command(
                    provider,
                    &connection.capability,
                    commands,
                    false,
                    &connection.consumer,
                    None,
                    langs,
                )
                .await
            {
                failure = Some(
                    match failure {
                        Some(previous) => error.with_note(langs, &previous),
                        None => error,
                    }
                    .to_string(),
                );
            }
        }
        self.dependencies.change(|state| {
            let consents = std::mem::take(&mut state.handoff_consents);
            *state = snapshot.0;
            state.handoff_consents = consents;
        })?;
        self.changed();
        match failure {
            Some(detail) => Err(InstallError::UpstreamUnavailable(detail)),
            None => Ok(()),
        }
    }

    pub(in crate::plugin) fn register_capability_provider(
        &self,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        let Some(provides) = manifest
            .provides
            .as_ref()
            .filter(|provides| !provides.capabilities.is_empty())
        else {
            return Ok(());
        };
        self.dependencies.change(|state| {
            let id = manifest.id.to_string();
            if !state.providers.contains(&id) {
                state.providers.push(id.clone());
            }
            for capability in provides.capabilities.keys() {
                state
                    .defaults
                    .entry(capability.clone())
                    .or_insert_with(|| id.clone());
            }
        })
    }

    pub(super) fn confirmed_install_targets(
        &self,
        manifest: &Manifest,
    ) -> Result<BTreeMap<String, String>, InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        Ok(super::declarations(manifest)
            .filter_map(|capability| {
                super::chosen(&state, &providers, &manifest.id, capability, None)
                    .map(|provider| (capability.clone(), provider.id.to_string()))
            })
            .collect())
    }
}

fn same_connection(before: &Connection, after: &Connection) -> bool {
    before.consumer == after.consumer
        && before.provider == after.provider
        && before.capability == after.capability
        && before.key == after.key
}
