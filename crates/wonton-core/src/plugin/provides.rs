//! 插件给其他插件提供的能力（`provides`）。

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError, Permission, PluginKind};
use crate::l10n::text;

pub use wonton_protocol::manifest::{
    CredentialCommands, EgressProxyProvider, ModelApis, ModelsProvider, ProviderService, Provides,
};

/// 凭据命令里的凭据名占位符。
pub const NAME_PLACEHOLDER: &str = "{name}";

fn invalid(message: &str) -> ManifestError {
    ManifestError::Invalid(message.to_owned())
}

/// 服务名是否存在由 [`super::package`] 对照 compose 检查。
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let egress = manifest
        .provides
        .as_ref()
        .and_then(|p| p.egress_proxy.as_ref());
    let permission = manifest.permissions.contains(&Permission::EgressProxy);
    match (egress, permission) {
        (Some(_), false) => {
            return Err(invalid(&text(
                langs,
                "plugin-install-prv-egress-needs-permission",
                &[],
            )));
        }
        (None, true) => {
            return Err(invalid(&text(
                langs,
                "plugin-install-prv-permission-needs-egress",
                &[],
            )));
        }
        _ => {}
    }
    if let Some(egress) = egress {
        if !manifest.permissions.contains(&Permission::NetworkEgress) {
            return Err(invalid(&text(
                langs,
                "plugin-install-prv-egress-needs-network",
                &[],
            )));
        }
        if egress.port == 0 {
            return Err(invalid(&text(langs, "plugin-install-prv-egress-port", &[])));
        }
    }
    let Some(provides) = &manifest.provides else {
        return Ok(());
    };
    if provides.models.is_none()
        && provides.egress_proxy.is_none()
        && provides.capabilities.is_empty()
    {
        return Err(invalid(&text(langs, "plugin-install-prv-empty", &[])));
    }
    let Some(models) = &provides.models else {
        return Ok(());
    };
    if manifest.kind != PluginKind::System {
        return Err(invalid(&text(
            langs,
            "plugin-install-prv-models-system-only",
            &[],
        )));
    }
    validate_models(models, langs)
}

fn validate_models(models: &ModelsProvider, langs: &Languages) -> Result<(), ManifestError> {
    if models.port == 0 {
        return Err(invalid(&text(langs, "plugin-install-prv-models-port", &[])));
    }
    let apis = [
        Some(&models.apis.openai),
        models.apis.anthropic.as_ref(),
        models.apis.gemini.as_ref(),
    ];
    if apis
        .into_iter()
        .flatten()
        .any(|path| !path.starts_with('/'))
    {
        return Err(invalid(&text(
            langs,
            "plugin-install-prv-models-apis-path",
            &[],
        )));
    }
    if models.default_model.trim().is_empty() {
        return Err(invalid(&text(
            langs,
            "plugin-install-prv-models-default-model",
            &[],
        )));
    }
    for (field, argv) in [
        ("issue", &models.credentials.issue),
        ("revoke", &models.credentials.revoke),
    ] {
        if !argv.iter().any(|arg| arg.contains(NAME_PLACEHOLDER)) {
            return Err(invalid(&text(
                langs,
                "plugin-install-prv-credentials-argv",
                &[
                    ("field", field.into()),
                    ("placeholder", NAME_PLACEHOLDER.into()),
                ],
            )));
        }
    }
    Ok(())
}
