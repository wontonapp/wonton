//! Remote MCP packages have no local runtime: reject unused fields rather than silently ignoring them.

use std::collections::BTreeSet;

use serde_json::Value;
use url::{Host, Url};

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError, Runtime};
use super::manifest_remote::RemoteAuth;
use super::secret_spec::SecretSpec;
use super::{manifest_mcp, proxy_auth};
use crate::l10n::text;

const FORBIDDEN_FIELDS: &[&str] = &[
    "webui",
    "tabs",
    "mcp",
    "bot",
    "bots",
    "agent",
    "provides",
    "requires",
    "wants",
    "gpu",
    "lan",
    "tunnel",
    "setup",
    "settings",
    "requirements",
    "optional_permissions",
    "devices",
    "clients",
    "matrix_widget",
    "static_web",
    "link",
    "install_timeout_secs",
];

/// Record presence before deserialization loses empty lists/maps and absent-vs-null fields.
/// `value` is the x-wonton object; `has_services` refers to the top-level compose key.
pub(super) fn raw_forbidden_fields(value: &Value, has_services: bool) -> Vec<&'static str> {
    if value.get("runtime").and_then(Value::as_str) != Some("remote_mcp") {
        return Vec::new();
    }
    let mut fields: Vec<_> = FORBIDDEN_FIELDS
        .iter()
        .copied()
        .filter(|field| value.get(*field).is_some())
        .collect();
    if has_services {
        fields.push("services");
    }
    fields
}

pub(super) fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    validate_package(manifest, &[], langs)
}

/// Package parsing adds raw-only fields before the ordinary manifest validators can fail early.
pub(super) fn validate_package(
    manifest: &Manifest,
    raw_fields: &[&str],
    langs: &Languages,
) -> Result<(), ManifestError> {
    let mut issues = Vec::new();
    if manifest.runtime != Runtime::RemoteMcp {
        if manifest.remote_mcp.is_some() {
            issues.push(text(langs, "remote-mcp-runtime-mismatch", &[]));
        }
    } else {
        forbidden_fields(manifest, raw_fields, langs, &mut issues);
        for (name, spec) in &manifest.secrets {
            if !matches!(spec, SecretSpec::Input(_)) {
                issues.push(text(
                    langs,
                    "remote-mcp-input-only",
                    &[("name", name.into())],
                ));
            }
        }
        match &manifest.remote_mcp {
            None => issues.push(text(langs, "remote-mcp-required", &[])),
            Some(remote) => validate_remote(manifest, remote, langs, &mut issues),
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::Invalid(issues.join("\n")))
    }
}

fn forbidden_fields(
    manifest: &Manifest,
    raw: &[&str],
    langs: &Languages,
    issues: &mut Vec<String>,
) {
    let present = [
        ("webui", manifest.webui.is_some()),
        ("tabs", manifest.tabs.is_some()),
        ("mcp", manifest.mcp.is_some()),
        ("bot", manifest.bot.is_some()),
        ("bots", manifest.bots.is_some()),
        ("agent", manifest.agent.is_some()),
        ("provides", manifest.provides.is_some()),
        ("requires", !manifest.requires.is_empty()),
        ("wants", !manifest.wants.is_empty()),
        ("gpu", manifest.gpu.is_some()),
        ("lan", manifest.lan.is_some()),
        ("tunnel", manifest.tunnel.is_some()),
        ("setup", manifest.setup.is_some()),
        ("settings", manifest.settings.is_some()),
        ("requirements", manifest.requirements.is_some()),
        (
            "optional_permissions",
            !manifest.optional_permissions.is_empty(),
        ),
        ("permissions", !manifest.permissions.is_empty()),
        ("devices", manifest.devices.is_some()),
        ("clients", manifest.clients.is_some()),
        ("matrix_widget", manifest.matrix_widget.is_some()),
        ("static_web", manifest.static_web.is_some()),
        ("link", manifest.link.is_some()),
        (
            "install_timeout_secs",
            manifest.install_timeout_secs.is_some(),
        ),
    ];
    let mut fields: BTreeSet<&str> = raw.iter().copied().collect();
    fields.extend(
        present
            .into_iter()
            .filter_map(|(field, present)| present.then_some(field)),
    );
    for field in fields {
        issues.push(text(
            langs,
            "remote-mcp-field-forbidden",
            &[("field", field.into())],
        ));
    }
}

fn validate_remote(
    manifest: &Manifest,
    remote: &super::manifest_remote::RemoteMcpSpec,
    langs: &Languages,
    issues: &mut Vec<String>,
) {
    if let Err(error) = check_url(manifest, &remote.url, langs) {
        issues.push(error);
    }
    match remote.auth {
        RemoteAuth::Header if remote.headers.is_empty() => {
            issues.push(text(langs, "remote-mcp-headers-required", &[]));
        }
        RemoteAuth::None | RemoteAuth::Oauth if !remote.headers.is_empty() => {
            issues.push(text(langs, "remote-mcp-headers-auth", &[]));
        }
        _ => {}
    }
    if remote.auth == RemoteAuth::Oauth && !manifest.secrets.is_empty() {
        issues.push(text(langs, "remote-mcp-oauth-secrets", &[]));
    }
    for (name, template) in &remote.headers {
        if let Err(error) = proxy_auth::validate_header_spec(name, template, langs) {
            issues.push(error);
        }
        if let Err(error) = proxy_auth::validate_input_template(manifest, template, langs) {
            issues.push(format!("remote_mcp.headers.{name}: {error}"));
        }
    }
    if let Err(error) = manifest_mcp::validate_limits(
        "remote_mcp",
        remote.tool_idle_seconds,
        remote.tool_total_seconds,
        remote.tools.as_deref(),
    ) {
        issues.push(error.to_string());
    }
}

fn check_url(manifest: &Manifest, template: &str, langs: &Languages) -> Result<(), String> {
    let invalid = || text(langs, "remote-mcp-url-invalid", &[]);
    let authority_start = template.find("://").ok_or_else(invalid)? + 3;
    let authority_end = template[authority_start..]
        .find(['/', '?', '#'])
        .map_or(template.len(), |end| authority_start + end);
    if template.chars().any(char::is_whitespace)
        || template.contains(['#', '\\'])
        || authority_start == authority_end
        || template[..authority_end].contains(['{', '}', '@'])
    {
        return Err(invalid());
    }
    let probe = proxy_auth::render_input_template_with(manifest, template, langs, |_, output| {
        output.push_str("remote-secret");
        Ok(())
    })?;
    let url = Url::parse(&probe).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    match url.host() {
        Some(Host::Domain(host)) => {
            let host = host.trim_end_matches('.');
            if !host.contains('.') || host == "localhost" || host.ends_with(".localhost") {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

#[cfg(test)]
#[path = "remote_validate_tests.rs"]
mod remote_validate_tests;
