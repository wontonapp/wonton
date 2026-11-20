//! Input-secret templates share proxy authentication's parser, without identity placeholders.

use super::{HeaderValue, Languages, Manifest, Part, SecretSpec, Secrets, parse, part_name, text};

pub(in crate::plugin) const URL_SECRET_COMPONENT: &percent_encoding::AsciiSet =
    &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');

/// Remote templates cannot use generated credentials or a caller's identity.
pub(in crate::plugin) fn validate_input_template(
    manifest: &Manifest,
    template: &str,
    langs: &Languages,
) -> Result<Vec<String>, String> {
    input_parts(manifest, template, langs).map(|parts| {
        parts
            .into_iter()
            .filter_map(|part| match part {
                Part::Secret(name) => Some(name),
                _ => None,
            })
            .collect()
    })
}

fn input_parts(
    manifest: &Manifest,
    template: &str,
    langs: &Languages,
) -> Result<Vec<Part>, String> {
    let parts = parse(template, langs)?;
    for part in &parts {
        match part {
            Part::Secret(name)
                if !matches!(manifest.secrets.get(name), Some(SecretSpec::Input(_))) =>
            {
                return Err(text(
                    langs,
                    "remote-mcp-input-only",
                    &[("name", name.into())],
                ));
            }
            Part::Text(_) | Part::Secret(_) => {}
            identity => {
                return Err(text(
                    langs,
                    "plugin-install-remote-identity-placeholder",
                    &[
                        ("placeholder", part_name(identity).into()),
                        ("template", template.into()),
                    ],
                ));
            }
        }
    }
    Ok(parts)
}

/// Callers may substitute harmless tokens to validate a URL without reading credentials.
pub(in crate::plugin) fn render_input_template_with(
    manifest: &Manifest,
    template: &str,
    langs: &Languages,
    mut resolve: impl FnMut(&str, &mut String) -> Result<(), String>,
) -> Result<String, String> {
    let mut value = String::new();
    for part in input_parts(manifest, template, langs)? {
        match part {
            Part::Text(literal) => value.push_str(&literal),
            Part::Secret(name) => resolve(&name, &mut value)?,
            _ => unreachable!("input_parts rejects identity placeholders"),
        }
    }
    Ok(value)
}

pub(in crate::plugin) fn render_input_url(
    manifest: &Manifest,
    secrets: &Secrets,
    template: &str,
    langs: &Languages,
) -> Result<String, String> {
    render_input_template_with(manifest, template, langs, |name, output| {
        output.extend(percent_encoding::utf8_percent_encode(
            input_value(secrets, name, langs)?,
            URL_SECRET_COMPONENT,
        ));
        Ok(())
    })
}

/// Values never enter an error: even a failed HTTP-header parse can contain a credential.
pub(in crate::plugin) fn render_input_header(
    manifest: &Manifest,
    secrets: &Secrets,
    template: &str,
    langs: &Languages,
) -> Result<Option<String>, String> {
    let mut missing_optional = false;
    let value = render_input_template_with(manifest, template, langs, |name, output| {
        if secrets.get(name).is_none()
            && matches!(manifest.secrets.get(name), Some(SecretSpec::Input(input)) if !input.required)
        {
            missing_optional = true;
            return Ok(());
        }
        output.push_str(input_value(secrets, name, langs)?);
        Ok(())
    })?;
    if missing_optional {
        return Ok(None);
    }
    check_header(&value, langs)?;
    Ok(Some(value))
}

fn input_value<'a>(secrets: &'a Secrets, name: &str, langs: &Languages) -> Result<&'a str, String> {
    let bytes = secrets.get(name).ok_or_else(|| {
        text(
            langs,
            "plugin-settings-secret-missing",
            &[("name", name.into())],
        )
    })?;
    std::str::from_utf8(bytes).map_err(|_| {
        text(
            langs,
            "plugin-settings-secret-not-text",
            &[("name", name.into())],
        )
    })
}

fn check_header(value: &str, langs: &Languages) -> Result<(), String> {
    HeaderValue::from_str(value)
        .map(|_| ())
        .map_err(|_| text(langs, "remote-mcp-header-value-invalid", &[]))
}
