use std::collections::BTreeSet;

use super::{InstallError, Languages, PluginId, PluginManager};
use crate::plugin::manager::UninstallReport;

impl PluginManager {
    pub(in crate::plugin) async fn uninstall_with_providers(
        &self,
        id: &PluginId,
        purge: bool,
        selected: Vec<String>,
        approve_connections: bool,
        langs: &Languages,
    ) -> Result<UninstallReport, InstallError> {
        self.check_uninstall_state(id, langs)?;
        let _changing = self.dependencies.changing.lock().await;
        let preview = self.uninstall_preview(id, langs)?;
        if !approve_connections {
            self.check_uninstall_connections(id, langs)?;
        }
        let mut seen = BTreeSet::new();
        let mut providers = Vec::with_capacity(selected.len());
        for selected in selected {
            if !seen.insert(selected.clone())
                || !preview
                    .unused_providers
                    .iter()
                    .any(|provider| provider.id == selected)
            {
                return Err(selection_error(langs));
            }
            let provider = PluginId::parse(&selected)
                .map_err(|error| InstallError::Invalid(error.to_string()))?;
            self.check_uninstall_state(&provider, langs)?;
            let dependents: Vec<_> = self
                .capability_dependents(&provider, None, langs)?
                .into_iter()
                .filter(|consumer| consumer.id != id.as_str())
                .collect();
            if !dependents.is_empty() {
                return Err(selection_error(langs));
            }
            providers.push(provider);
        }
        let mut report = self
            .uninstall_locked(id, purge, approve_connections, langs)
            .await?;
        for provider in providers {
            let removed = self
                .uninstall_locked(&provider, purge, approve_connections, langs)
                .await?;
            report.containers.extend(removed.containers);
            report.networks.extend(removed.networks);
            report.volumes.extend(removed.volumes);
            report.bots.extend(removed.bots);
        }
        Ok(report)
    }

    pub(in crate::plugin) fn check_uninstall_connections(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        for consumer in self.capability_consumers() {
            for capability in consumer
                .manifest
                .requires
                .iter()
                .filter(|capability| super::generic(capability))
            {
                if super::chosen(&state, &providers, &consumer.manifest.id, capability)
                    .is_none_or(|provider| provider.id != *id)
                {
                    continue;
                }
                let replacement = match state
                    .defaults
                    .get(capability)
                    .filter(|provider| provider.as_str() != id.as_str())
                {
                    Some(default) => providers.iter().find(|provider| {
                        provider.id.as_str() == default
                            && super::service(&provider.listed.manifest, capability).is_some()
                    }),
                    None => providers.iter().find(|provider| {
                        provider.id != *id
                            && super::service(&provider.listed.manifest, capability).is_some()
                    }),
                };
                if let Some(provider) = replacement
                    && super::high(&provider.listed.manifest, capability)
                    && super::connection(&state, &consumer.manifest.id, &provider.id, capability)
                        .is_none()
                {
                    return Err(InstallError::PermissionNotGranted(crate::l10n::text(
                        langs,
                        "plugin-capability-connect",
                        &[
                            (
                                "consumer",
                                langs.pick_owned(&consumer.manifest.title).into(),
                            ),
                            (
                                "provider",
                                langs.pick_owned(&provider.listed.manifest.title).into(),
                            ),
                            ("capability", capability.as_str().into()),
                        ],
                    )));
                }
            }
        }
        Ok(())
    }
}

fn selection_error(langs: &Languages) -> InstallError {
    InstallError::Invalid(crate::l10n::text(
        langs,
        "plugin-capability-uninstall-selection",
        &[],
    ))
}
