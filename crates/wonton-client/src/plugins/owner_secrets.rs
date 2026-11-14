use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::task::AbortHandle;
use wonton_l10n::l10n;
use wonton_protocol::{
    Action, CoreError, OwnerPluginSecretValue, OwnerPluginSecrets, OwnerSecretFailure,
    OwnerSecretProblem, OwnerSecretView, OwnerSecretsView, PluginFailure, PluginProblem,
};

use super::api::CoreApiError;
use super::{PluginsLink, READ_TIMEOUT, lock};
use crate::l10n::{CATALOG, language};

#[derive(Default)]
pub(super) struct Local {
    epoch: u64,
    request: u64,
    list: Option<AbortHandle>,
    rows: HashMap<String, (u64, AbortHandle)>,
}

impl Local {
    fn clear(&mut self) {
        self.epoch += 1;
        if let Some(list) = self.list.take() {
            list.abort();
        }
        for (_, (_, task)) in self.rows.drain() {
            task.abort();
        }
    }
}

impl PluginsLink {
    pub(crate) fn dispatch_owner_secrets(
        self: &Arc<Self>,
        action: Action,
    ) -> Result<(), CoreError> {
        match action {
            Action::ListOwnerSecrets { plugin_id } => self.list_owner_secrets(plugin_id),
            Action::ShowOwnerSecret { plugin_id, name } => {
                return self.show_owner_secret(plugin_id, name);
            }
            Action::HideOwnerSecret { plugin_id, name } => {
                self.hide_owner_secret(&plugin_id, &name)
            }
            Action::DismissOwnerSecrets { plugin_id } => {
                let mut local = lock(&self.owner_secrets);
                if self
                    .view
                    .borrow()
                    .owner_secrets
                    .as_ref()
                    .is_some_and(|view| view.plugin_id == plugin_id)
                {
                    local.clear();
                    self.view.send_modify(|view| view.owner_secrets = None);
                }
            }
            _ => unreachable!("only owner secret actions are routed here"),
        }
        Ok(())
    }

    fn list_owner_secrets(self: &Arc<Self>, plugin_id: String) {
        let mut local = lock(&self.owner_secrets);
        local.clear();
        let epoch = local.epoch;
        let generation = self.generation.load(Ordering::SeqCst);
        self.view.send_modify(|view| {
            view.owner_secrets = Some(OwnerSecretsView {
                plugin_id: plugin_id.clone(),
                loading: true,
                secrets: Vec::new(),
                problem: None,
            });
        });
        let link = Arc::clone(self);
        let task = self.runtime.spawn(async move {
            let fetched = match link.api() {
                Ok(api) => api
                    .get_detailed::<OwnerPluginSecrets>(
                        &["plugins", &plugin_id, "secrets"],
                        &[],
                        READ_TIMEOUT,
                    )
                    .await
                    .map_err(problem),
                Err(error) => Err(connection_problem(error)),
            };
            let mut local = lock(&link.owner_secrets);
            if local.epoch != epoch || !link.is_current(generation) {
                return;
            }
            local.list = None;
            link.view.send_modify(|view| {
                let Some(group) = view.owner_secrets.as_mut() else {
                    return;
                };
                group.loading = false;
                match fetched {
                    Ok(list) => {
                        group.secrets = list
                            .secrets
                            .into_iter()
                            .map(|secret| OwnerSecretView {
                                name: secret.name,
                                label: secret.label,
                                expanded: false,
                                loading: false,
                                value: None,
                                problem: None,
                            })
                            .collect();
                    }
                    Err(error) => group.problem = Some(error),
                }
            });
        });
        local.list = Some(task.abort_handle());
    }

    fn show_owner_secret(
        self: &Arc<Self>,
        plugin_id: String,
        name: String,
    ) -> Result<(), CoreError> {
        let mut local = lock(&self.owner_secrets);
        let listed = self
            .view
            .borrow()
            .owner_secrets
            .as_ref()
            .is_some_and(|group| {
                group.plugin_id == plugin_id && group.secrets.iter().any(|row| row.name == name)
            });
        if !listed {
            return Err(CoreError::InvalidInput {
                reason: l10n!(
                    CATALOG,
                    language(),
                    "plugins-owner-secret-not-listed",
                    name = name.as_str()
                ),
            });
        }
        if let Some((_, task)) = local.rows.remove(&name) {
            task.abort();
        }
        local.request += 1;
        let request = local.request;
        let epoch = local.epoch;
        let generation = self.generation.load(Ordering::SeqCst);
        self.view.send_modify(|view| {
            let row = view
                .owner_secrets
                .as_mut()
                .unwrap()
                .secrets
                .iter_mut()
                .find(|row| row.name == name)
                .unwrap();
            row.expanded = true;
            row.loading = true;
            row.value = None;
            row.problem = None;
        });
        let link = Arc::clone(self);
        let secret_name = name.clone();
        let task =
            self.runtime.spawn(async move {
                let fetched = match link.api() {
                    Ok(api) => api
                        .get_detailed::<OwnerPluginSecretValue>(
                            &["plugins", &plugin_id, "secrets", &secret_name],
                            &[],
                            READ_TIMEOUT,
                        )
                        .await
                        .map_err(problem),
                    Err(error) => Err(connection_problem(error)),
                };
                let mut local = lock(&link.owner_secrets);
                if local.epoch != epoch
                    || !link.is_current(generation)
                    || !local
                        .rows
                        .get(&secret_name)
                        .is_some_and(|(current, _)| *current == request)
                {
                    return;
                }
                local.rows.remove(&secret_name);
                link.view.send_modify(|view| {
                    let Some(row) = view.owner_secrets.as_mut().and_then(|group| {
                        group.secrets.iter_mut().find(|row| row.name == secret_name)
                    }) else {
                        return;
                    };
                    row.loading = false;
                    match fetched {
                        Ok(secret) => row.value = Some(secret.value),
                        Err(error) => row.problem = Some(error),
                    }
                });
            });
        local.rows.insert(name, (request, task.abort_handle()));
        Ok(())
    }

    fn hide_owner_secret(&self, plugin_id: &str, name: &str) {
        let mut local = lock(&self.owner_secrets);
        if !self
            .view
            .borrow()
            .owner_secrets
            .as_ref()
            .is_some_and(|group| group.plugin_id == plugin_id)
        {
            return;
        }
        if let Some((_, task)) = local.rows.remove(name) {
            task.abort();
        }
        self.view.send_modify(|view| {
            if let Some(row) = view
                .owner_secrets
                .as_mut()
                .and_then(|group| group.secrets.iter_mut().find(|row| row.name == name))
            {
                row.expanded = false;
                row.loading = false;
                row.value = None;
                row.problem = None;
            }
        });
    }

    pub(super) fn forget_owner_secrets(&self) {
        let mut local = lock(&self.owner_secrets);
        local.clear();
        self.view
            .send_if_modified(|view| view.owner_secrets.take().is_some());
    }
}

fn connection_problem(error: PluginProblem) -> OwnerSecretProblem {
    OwnerSecretProblem {
        reason: match error.reason {
            PluginFailure::TransportUnavailable => OwnerSecretFailure::TransportUnavailable,
            PluginFailure::Unauthorized => OwnerSecretFailure::Unauthenticated,
            _ => OwnerSecretFailure::Protocol,
        },
        message: error.message,
    }
}

fn problem(error: CoreApiError) -> OwnerSecretProblem {
    match error {
        CoreApiError::Transport(error) => connection_problem(error),
        CoreApiError::Protocol(message) => OwnerSecretProblem {
            reason: OwnerSecretFailure::Protocol,
            message,
        },
        CoreApiError::Http {
            status,
            kind,
            message,
            ..
        } => {
            let reason = match kind.as_deref() {
                Some("unauthenticated") => OwnerSecretFailure::Unauthenticated,
                Some("cross_site") => OwnerSecretFailure::CrossSite,
                Some("permission_required") => OwnerSecretFailure::PermissionRequired,
                Some("not_installed") => OwnerSecretFailure::NotInstalled,
                Some("not_found") => OwnerSecretFailure::NotFound,
                Some("not_ready") => OwnerSecretFailure::NotReady,
                Some(_) if status >= 500 => OwnerSecretFailure::Upstream,
                Some(_) => OwnerSecretFailure::Other,
                None if status == 401 => OwnerSecretFailure::Unauthenticated,
                None if status == 403 || status == 502 => OwnerSecretFailure::TransportUnavailable,
                None if status >= 500 => OwnerSecretFailure::Upstream,
                None => OwnerSecretFailure::Protocol,
            };
            OwnerSecretProblem { reason, message }
        }
    }
}
