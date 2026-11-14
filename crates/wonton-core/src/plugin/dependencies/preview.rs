use std::collections::BTreeSet;

use wonton_protocol::{
    CapabilityProvider, CapabilityProviderRef, CardLine, MissingCapability, SourceKind,
};

use super::{InstallError, Languages, Manifest, PluginManager, chosen, generic, high, service};
use crate::l10n;
use crate::plugin::github_source::{Origin, Trust};
use crate::plugin::index_location::IndexLocation;
use crate::plugin::storefront::index::{Files, Listed};

pub(in crate::plugin) struct DependencyPreview {
    pub missing_requires: Vec<MissingCapability>,
    pub missing_wants: Vec<MissingCapability>,
    pub wanting_plugins: Vec<CapabilityProviderRef>,
    pub notices: Vec<CardLine>,
    pub blocked: Option<String>,
}

pub(super) struct Candidate {
    pub listed: Listed,
    pub files: Files,
    pub origin: Origin,
}

impl Candidate {
    pub fn source(&self) -> String {
        self.origin
            .index
            .clone()
            .unwrap_or_else(|| format!("{}@{}", self.listed.entry.source, self.listed.entry.commit))
    }
}

impl PluginManager {
    pub(super) async fn dependency_candidates(
        &self,
        langs: &Languages,
    ) -> Result<Vec<Candidate>, InstallError> {
        let (index, _, _) = self.storefront.official(false, langs).await;
        let mut candidates: Vec<_> = index
            .plugins
            .into_iter()
            .map(|listed| Candidate {
                origin: listed.origin(),
                listed,
                files: self.storefront.files(),
            })
            .collect();
        self.storefront.added.refresh(false, langs).await;
        for source in self
            .storefront
            .added
            .views()
            .into_iter()
            .filter(|s| s.kind == SourceKind::Index)
        {
            for plugin in source.plugins {
                let (location, id) = IndexLocation::of_item_source(&plugin.source, langs)?;
                let listed = self
                    .storefront
                    .added
                    .listed_in(&location, id, langs)
                    .await?;
                let mut origin = listed.origin();
                origin.trust = Trust::Community;
                origin.index = Some(plugin.source);
                candidates.push(Candidate {
                    listed,
                    origin,
                    files: Files::Third {
                        location,
                        github: self.github.clone(),
                    },
                });
            }
        }
        let mut seen = BTreeSet::new();
        candidates.retain(|c| seen.insert((c.listed.manifest.id.clone(), c.source())));
        Ok(candidates)
    }

    pub(in crate::plugin) async fn dependency_preview(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<DependencyPreview, InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        let missing: Vec<_> = super::required(manifest)
            .filter(|c| generic(c))
            .filter(|c| chosen(&state, &providers, &manifest.id, c, None).is_none())
            .collect();
        let candidates = if missing.is_empty() {
            Vec::new()
        } else {
            self.dependency_candidates(langs).await?
        };
        let mut preview = DependencyPreview {
            missing_requires: Vec::new(),
            missing_wants: Vec::new(),
            wanting_plugins: self.wanting_capability_consumers(manifest, langs)?,
            notices: Vec::new(),
            blocked: None,
        };
        for consumer in self.capability_consumers().iter().filter(|c| {
            preview
                .wanting_plugins
                .iter()
                .any(|p| p.id == c.manifest.id.as_str())
        }) {
            for capability in consumer
                .manifest
                .wants
                .iter()
                .filter(|c| generic(c) && service(manifest, c).is_some() && high(manifest, c))
            {
                preview.notices.push(connection_notice(
                    &consumer.manifest,
                    manifest,
                    capability,
                    langs,
                ));
            }
        }
        for capability in missing {
            let matches: Vec<_> = candidates
                .iter()
                .filter(|c| {
                    c.listed.manifest.id != manifest.id
                        && service(&c.listed.manifest, capability).is_some()
                })
                .collect();
            if matches.is_empty() && preview.blocked.is_none() {
                preview.blocked = Some(self.capability_missing_other(manifest, capability, langs));
            }
            preview.missing_requires.push(MissingCapability {
                capability: capability.to_owned(),
                third_party: crate::plugin::capabilities::is_third_party(capability),
                providers: matches
                    .iter()
                    .map(|c| CapabilityProvider {
                        id: c.listed.manifest.id.to_string(),
                        title: langs.pick_owned(&c.listed.manifest.title),
                        source: c.source(),
                        high_privilege: high(&c.listed.manifest, capability),
                    })
                    .collect(),
            });
        }
        for capability in manifest.wants.iter().filter(|c| generic(c)) {
            if chosen(&state, &providers, &manifest.id, capability, None).is_none() {
                preview.missing_wants.push(MissingCapability {
                    capability: capability.clone(),
                    third_party: crate::plugin::capabilities::is_third_party(capability),
                    providers: Vec::new(),
                });
            }
        }
        for capability in super::declarations(manifest) {
            if let Some(provider) = chosen(&state, &providers, &manifest.id, capability, None)
                .filter(|p| high(&p.listed.manifest, capability))
            {
                preview.notices.push(connection_notice(
                    manifest,
                    &provider.listed.manifest,
                    capability,
                    langs,
                ));
            }
        }
        Ok(preview)
    }

    pub(super) fn wanting_capability_consumers(
        &self,
        provider: &Manifest,
        langs: &Languages,
    ) -> Result<Vec<CapabilityProviderRef>, InstallError> {
        let state = self.dependencies.load()?;
        let providers = self.capability_providers();
        Ok(self
            .capability_consumers()
            .iter()
            .filter(|consumer| {
                consumer.manifest.id != provider.id
                    && consumer.manifest.wants.iter().any(|c| {
                        generic(c)
                            && service(provider, c).is_some()
                            && high(provider, c)
                            && chosen(&state, &providers, &consumer.manifest.id, c, None)
                                .is_none_or(|p| p.id == provider.id)
                            && super::connection(&state, &consumer.manifest.id, &provider.id, c)
                                .is_none()
                    })
            })
            .map(|consumer| CapabilityProviderRef {
                id: consumer.manifest.id.to_string(),
                title: langs.pick_owned(&consumer.manifest.title),
                high_privilege: true,
            })
            .collect())
    }
}

fn connection_notice(
    consumer: &Manifest,
    provider: &Manifest,
    capability: &str,
    langs: &Languages,
) -> CardLine {
    CardLine {
        text: l10n::text(
            langs,
            "plugin-capability-connect",
            &[
                ("consumer", langs.pick_owned(&consumer.title).into()),
                ("provider", langs.pick_owned(&provider.title).into()),
                ("capability", capability.into()),
            ],
        ),
        emphasis: true,
    }
}
