use std::collections::BTreeSet;
use std::sync::Arc;

use wonton_protocol::{CardLine, InstallPreview, InstallRequest, InstallTogether, SecretValues};

use super::{ToolFailure, cards};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::privileges::{self, CapabilityRisk};
use crate::plugin::{InstallError, PluginManager, PluginPackage};

pub(super) struct Proposed {
    pub request: InstallRequest,
    pub second: Option<cards::Draft>,
}

impl Proposed {
    pub fn with_secrets(mut self, secrets: SecretValues) -> InstallRequest {
        for (name, value) in secrets {
            if let Some((id, input)) = name.split_once('/')
                && let Some(provider) = self
                    .request
                    .install_together
                    .iter_mut()
                    .find(|p| p.id == id)
            {
                provider.request.secrets.insert(input.to_owned(), value);
            } else {
                self.request.secrets.insert(name, value);
            }
        }
        // Manifest input names only contain [a-z0-9_], so a provider's '/' namespace cannot
        // collide with a consumer input. The card validator already rejected unknown names.
        self.request
    }
}

fn failure(error: InstallError) -> ToolFailure {
    ToolFailure::new(error.kind(), error.to_string())
}

fn request(preview: &InstallPreview, langs: &Languages) -> Result<InstallRequest, ToolFailure> {
    if let Some(blocked) = &preview.blocked {
        return Err(ToolFailure::new("invalid_input", blocked.clone()));
    }
    let source = preview.source.clone().ok_or_else(|| {
        ToolFailure::new(
            "invalid_input",
            l10n::text(langs, "plugin-capability-request-format", &[]),
        )
    })?;
    Ok(InstallRequest {
        source,
        digest: preview.digest.clone(),
        plan: preview.plan.clone(),
        scope: None,
        grant: preview
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.permission.clone())
            .collect(),
        enabled_permissions: Vec::new(),
        secrets: SecretValues::new(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
    })
}

/// A deterministic proposal is disclosed on the owner card; no installation or secret
/// write happens until the owner has approved every required confirmation.
pub(super) async fn propose(
    plugins: &Arc<PluginManager>,
    package: &PluginPackage,
    draft: &mut cards::Draft,
    mut second: Option<cards::Draft>,
    langs: &Languages,
) -> Result<Proposed, ToolFailure> {
    let mut preview = plugins
        .preview_install(package, langs)
        .await
        .map_err(failure)?;
    if preview.source.is_none() {
        // Staging gives the request a source without changing the actual outbox package's provenance.
        preview = plugins
            .preview_upload(package.clone(), langs)
            .await
            .map_err(failure)?;
    }
    let mut request = request(&preview, langs)?;
    let mut selected = BTreeSet::new();
    for missing in &preview.missing_requires {
        let provider = missing.providers.first().ok_or_else(|| {
            ToolFailure::new(
                "invalid_input",
                l10n::text(
                    langs,
                    "plugin-capability-missing",
                    &[("capability", missing.capability.as_str().into())],
                ),
            )
        })?;
        if !selected.insert(provider.id.clone()) {
            continue;
        }
        let provider_preview = plugins
            .preview_source(&provider.source, langs)
            .await
            .map_err(failure)?;
        let mut provider_request = self::request(&provider_preview, langs)?;
        // Official previews display a release tag; the candidate's commit is the selection we bind.
        provider_request.source = provider.source.clone();
        let provider_package = plugins
            .fetch_source(&provider_request.source)
            .await
            .map_err(failure)?;
        if provider_preview.id != provider.id
            || provider_package.manifest.id.as_str() != provider.id
            || provider_package.digest() != provider_request.digest
        {
            return Err(ToolFailure::new("changed", provider.source.clone()));
        }
        disclose_provider(draft, &provider_preview, &provider_request, langs);
        for mut input in provider_preview.secret_inputs {
            input.name = format!("{}/{}", provider.id, input.name);
            input.title = format!("{} · {}", provider_preview.title, input.title);
            draft.secret_inputs.push(input);
        }
        if provider_package
            .manifest
            .permissions
            .iter()
            .any(|p| privileges::risk(p) == Some(CapabilityRisk::Highest))
        {
            let provider_second = cards::highest_confirm(&provider_package, langs);
            match &mut second {
                Some(second) => second.lines.extend(provider_second.lines),
                None => second = Some(provider_second),
            }
        }
        request.install_together.push(InstallTogether {
            id: provider.id.clone(),
            request: provider_request,
        });
    }
    disclose_defaults(draft, &preview, langs);
    draft.lines.extend(preview.notices);
    Ok(Proposed { request, second })
}

fn disclose_provider(
    draft: &mut cards::Draft,
    preview: &InstallPreview,
    request: &InstallRequest,
    langs: &Languages,
) {
    draft.lines.push(CardLine {
        text: l10n::text(
            langs,
            "plugin-capability-provider-install",
            &[
                ("provider", preview.title.as_str().into()),
                ("id", preview.id.as_str().into()),
            ],
        ),
        emphasis: true,
    });
    draft.lines.push(cards::line(request.source.clone()));
    for permission in &preview.permissions {
        draft.lines.push(CardLine {
            text: l10n::text(
                langs,
                "plugin-capability-provider-permission",
                &[
                    ("provider", preview.title.as_str().into()),
                    ("permission", permission.text.as_str().into()),
                ],
            ),
            emphasis: !permission.optional && permission.risk.is_some(),
        });
        if let Some(notice) = &permission.notice {
            draft.lines.push(CardLine {
                text: notice.clone(),
                emphasis: !permission.optional,
            });
        }
        if let Some(loss) = &permission.off_loss {
            draft.lines.push(cards::line(l10n::text(
                langs,
                "chat-card-permission-off-loss",
                &[("loss", loss.as_str().into())],
            )));
        }
    }
    disclose_defaults(draft, preview, langs);
    draft.lines.extend(preview.notices.iter().cloned());
}

fn disclose_defaults(draft: &mut cards::Draft, preview: &InstallPreview, langs: &Languages) {
    if preview.permissions.iter().any(|p| p.optional) {
        draft.lines.push(cards::line(l10n::text(
            langs,
            "plugin-capability-provider-optional-off",
            &[("provider", preview.title.as_str().into())],
        )));
    }
    if preview.lan_webui.is_some() {
        draft.lines.push(cards::line(format!(
            "{} · {}",
            preview.title,
            l10n::text(langs, "plugin-settings-lan-client-not-published", &[])
        )));
    }
}
