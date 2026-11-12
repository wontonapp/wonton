use super::{
    Connection, DependencySnapshot, InstallError, Languages, Manifest, PluginId, PluginManager,
    declarations, service,
};

impl PluginManager {
    pub(in crate::plugin) fn snapshot_dependency_connections(
        &self,
        id: &PluginId,
    ) -> Result<DependencySnapshot, InstallError> {
        Ok(DependencySnapshot(
            self.dependencies
                .load()?
                .connections
                .into_iter()
                .filter(|c| c.consumer == id.as_str())
                .collect(),
        ))
    }

    pub(in crate::plugin) async fn restore_dependency_connections(
        &self,
        id: &PluginId,
        snapshot: DependencySnapshot,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let now = self.dependencies.load()?;
        let added: Vec<_> = now
            .connections
            .iter()
            .filter(|c| {
                c.consumer == id.as_str()
                    && !snapshot.0.iter().any(|before| {
                        before.provider == c.provider && before.capability == c.capability
                    })
            })
            .cloned()
            .collect();
        self.revoke_connections(&added, langs).await?;
        self.dependencies.change(|state| {
            state.connections.retain(|c| c.consumer != id.as_str());
            state.connections.extend(snapshot.0);
        })
    }

    pub(in crate::plugin) async fn prune_dependency_connections(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let state = self.dependencies.load()?;
        let removed: Vec<_> = state
            .connections
            .iter()
            .filter(|c| {
                c.consumer == manifest.id.as_str()
                    && !declarations(manifest).any(|name| name == &c.capability)
            })
            .cloned()
            .collect();
        self.revoke_connections(&removed, langs).await?;
        self.dependencies.change(|state| {
            state.connections.retain(|c| {
                c.consumer != manifest.id.as_str()
                    || declarations(manifest).any(|name| name == &c.capability)
            });
            if let Some(selected) = state.selected.get_mut(manifest.id.as_str()) {
                selected
                    .retain(|name, _| declarations(manifest).any(|capability| capability == name));
            }
        })
    }

    async fn revoke_connections(
        &self,
        connections: &[Connection],
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let providers = self.capability_providers();
        for c in connections {
            let Some(p) = providers.iter().find(|p| p.id.as_str() == c.provider) else {
                continue;
            };
            let Some(commands) =
                service(&p.listed.manifest, &c.capability).and_then(|s| s.credentials.as_ref())
            else {
                continue;
            };
            self.capability_command(p, &c.capability, commands, false, &c.consumer, None, langs)
                .await?;
        }
        Ok(())
    }
}
