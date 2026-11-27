use std::collections::BTreeMap;

use hyper::header::{HeaderName, HeaderValue};

use super::spec::{FORBIDDEN_HEADERS, Part, is_cookie_name, is_cookie_value, parse};
use crate::l10n::text;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::{Manifest, ManifestError, SecretSpec};

/// 代登录用的密钥至少这么长：网关按值从响应里找出并去掉它，值太短会误伤别的内容。
const MIN_SECRET_BYTES: usize = 16;

pub(super) fn secret_ok(manifest: &Manifest, name: &str) -> bool {
    matches!(
        manifest.secrets.get(name),
        Some(SecretSpec::Generate(g)) if g.bytes >= MIN_SECRET_BYTES
    )
}

pub(super) fn unknown_secret(langs: &Languages, name: &str) -> String {
    text(
        langs,
        "plugin-install-proxy-secret-unknown",
        &[("name", name.into()), ("min", MIN_SECRET_BYTES.into())],
    )
}

pub(super) fn secret_missing(langs: &Languages, name: &str) -> String {
    text(
        langs,
        "plugin-install-proxy-secret-missing",
        &[("name", name.into())],
    )
}

/// 清单的代登录模板会不会把主人的身份带给应用。
pub fn uses_identity(manifest: &Manifest) -> bool {
    any_part(manifest, |part| {
        matches!(part, Part::User | Part::Name | Part::Email | Part::Role)
    })
}

fn any_part(manifest: &Manifest, matches: impl Fn(&Part) -> bool) -> bool {
    // 语言只影响报错文字，这里只看模板能不能解析。
    let langs = Languages::default();
    let Some(proxy) = manifest.webui.as_ref().and_then(|w| w.proxy.as_ref()) else {
        return false;
    };
    proxy
        .headers
        .values()
        .chain(proxy.cookies.values())
        .any(|template| parse(template, &langs).is_ok_and(|parts| parts.iter().any(&matches)))
}

fn invalid(message: String) -> ManifestError {
    ManifestError::Invalid(message)
}

/// 清单校验：请求头名和 Cookie 名合法；模板写法对，`{secret:…}` 只引用插件自己的 `secrets` 里由核心生成、
/// 至少 16 字节的项；Cookie 的值只允许 `{secret:…}`、`{role}` 和 cookie-octet 固定文字
/// （用户名、显示名和邮箱可能有 Cookie 里不能出现的字符）。
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let Some(proxy) = manifest.webui.as_ref().and_then(|w| w.proxy.as_ref()) else {
        return Ok(());
    };
    validate_headers(manifest, &proxy.headers, langs)?;
    let check_template = |template: &str, in_cookie: bool| -> Result<(), ManifestError> {
        for part in parse(template, langs).map_err(invalid)? {
            match part {
                Part::Secret(name) if !secret_ok(manifest, &name) => {
                    return Err(invalid(unknown_secret(langs, &name)));
                }
                Part::User | Part::Name | Part::Email if in_cookie => {
                    return Err(invalid(text(
                        langs,
                        "plugin-install-proxy-cookie-template",
                        &[("template", template.into())],
                    )));
                }
                Part::Text(text) if in_cookie && !is_cookie_value(&text) => {
                    return Err(invalid(cookie_value_error(langs, template)));
                }
                Part::Text(text) if HeaderValue::from_str(&text).is_err() => {
                    return Err(invalid(control_chars_error(langs, template)));
                }
                _ => {}
            }
        }
        Ok(())
    };
    for (name, template) in &proxy.cookies {
        if !is_cookie_name(name) || name.to_ascii_lowercase().starts_with("wonton_") {
            return Err(invalid(text(
                langs,
                "plugin-install-proxy-cookie-forbidden",
                &[("name", name.into())],
            )));
        }
        check_template(template, true)?;
    }
    Ok(())
}

/// 代登录请求头（`webui.proxy.headers`、`clients.http.auth.gateway.headers`）的校验：名字合法、不是网关
/// 自己设置的头，值模板里的每一段都能原样放进响应头。
pub fn validate_headers(
    manifest: &Manifest,
    headers: &BTreeMap<String, String>,
    langs: &Languages,
) -> Result<(), ManifestError> {
    for (name, template) in headers {
        let lower = name.to_ascii_lowercase();
        if HeaderName::from_bytes(name.as_bytes()).is_err()
            || FORBIDDEN_HEADERS.contains(&lower.as_str())
            || lower.starts_with("x-forwarded-")
        {
            return Err(invalid(text(
                langs,
                "plugin-install-proxy-header-forbidden",
                &[("name", name.into())],
            )));
        }
        for part in parse(template, langs).map_err(invalid)? {
            match part {
                Part::Secret(name) if !secret_ok(manifest, &name) => {
                    return Err(invalid(unknown_secret(langs, &name)));
                }
                Part::Text(text) if HeaderValue::from_str(&text).is_err() => {
                    return Err(invalid(control_chars_error(langs, template)));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn cookie_value_error(langs: &Languages, template: &str) -> String {
    text(
        langs,
        "plugin-install-proxy-cookie-chars",
        &[("template", template.into())],
    )
}

pub(super) fn control_chars_error(langs: &Languages, template: &str) -> String {
    text(
        langs,
        "plugin-install-proxy-header-control-chars",
        &[("template", template.into())],
    )
}
