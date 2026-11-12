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
        let mut seen = BTreeSet::new();
        let mut removing = Vec::with_capacity(selected.len() + 1);
        removing.push(id.clone());
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
            removing.push(provider);
        }
        self.check_uninstall_dependencies(&removing, approve_connections, langs)?;
        let mut report = self
            .uninstall_locked(id, purge, &removing, approve_connections, langs)
            .await?;
        for provider in removing.iter().skip(1) {
            let removed = self
                .uninstall_locked(provider, purge, &removing, approve_connections, langs)
                .await?;
            report.containers.extend(removed.containers);
            report.networks.extend(removed.networks);
            report.volumes.extend(removed.volumes);
            report.bots.extend(removed.bots);
        }
        Ok(report)
    }

    pub(in crate::plugin) fn check_uninstall_dependencies(
        &self,
        removing: &[PluginId],
        approve_connections: bool,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let mut blocked = Vec::new();
        let mut approval = None;
        for consumer in self.capability_consumers() {
            if removing.contains(&consumer.manifest.id) {
                continue;
            }
            let mut missing = false;
            for capability in consumer
                .manifest
                .requires
                .iter()
                .filter(|capability| super::generic(capability))
            {
                if super::chosen(&state, &providers, &consumer.manifest.id, capability)
                    .is_none_or(|provider| !removing.contains(&provider.id))
                {
                    continue;
                }
                let Some(provider) = remaining_provider(
                    &state,
                    &providers,
                    &consumer.manifest.id,
                    capability,
                    removing,
                ) else {
                    missing = true;
                    continue;
                };
                if !approve_connections
                    && super::high(&provider.listed.manifest, capability)
                    && super::connection(&state, &consumer.manifest.id, &provider.id, capability)
                        .is_none()
                {
                    approval = Some(InstallError::PermissionNotGranted(crate::l10n::text(
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
            if missing {
                blocked.push(wonton_protocol::CapabilityProviderRef {
                    id: consumer.manifest.id.to_string(),
                    title: langs.pick_owned(&consumer.manifest.title),
                    high_privilege: false,
                });
            }
        }
        if !blocked.is_empty() {
            let titles = blocked
                .iter()
                .map(|plugin| plugin.title.as_str())
                .collect::<Vec<_>>();
            return Err(InstallError::InUse {
                message: crate::l10n::text(
                    langs,
                    "plugin-capability-in-use",
                    &[("plugins", crate::l10n::list(langs, &titles).into())],
                ),
                plugins: blocked,
            });
        }
        approval.map_or(Ok(()), Err)
    }
}

pub(super) fn remaining_provider<'a>(
    state: &super::State,
    providers: &'a [super::Provider],
    consumer: &PluginId,
    capability: &str,
    removing: &[PluginId],
) -> Option<&'a super::Provider> {
    let retained = |id: &&String| {
        !removing
            .iter()
            .any(|removed| removed.as_str() == id.as_str())
    };
    let selected = state
        .selected
        .get(consumer.as_str())
        .and_then(|choices| choices.get(capability))
        .filter(retained)
        .or_else(|| state.defaults.get(capability).filter(retained));
    providers.iter().find(|provider| {
        !removing.contains(&provider.id)
            && super::service(&provider.listed.manifest, capability).is_some()
            && selected.is_none_or(|selected| selected == provider.id.as_str())
    })
}

fn selection_error(langs: &Languages) -> InstallError {
    InstallError::Invalid(crate::l10n::text(
        langs,
        "plugin-capability-uninstall-selection",
        &[],
    ))
}
