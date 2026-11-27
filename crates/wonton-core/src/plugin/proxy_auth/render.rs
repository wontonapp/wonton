use std::collections::BTreeMap;

use hyper::header::HeaderName;

use super::spec::{Part, parse};
use super::validate::{secret_missing, secret_ok, unknown_secret};
use crate::l10n::text;
use crate::owner::{Owner, internal_email};
use crate::plugin::languages::Languages;
use crate::plugin::manifest::{Manifest, Sso};

/// 解析好的代登录凭据：请求头和 Cookie 的名字及值模板，`{secret:…}` 已经换成核心生成的值。`Debug` 只打印名字。
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyAuth {
    headers: Vec<(HeaderName, Vec<Part>)>,
    cookies: Vec<(String, Vec<Part>)>,
}

impl std::fmt::Debug for ProxyAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<&str> = self.headers.iter().map(|(n, _)| n.as_str()).collect();
        let cookies: Vec<&str> = self.cookies.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("ProxyAuth")
            .field("headers", &headers)
            .field("cookies", &cookies)
            .finish()
    }
}

impl ProxyAuth {
    /// 声明了 `sso: proxy` 的插件的凭据；`generated` 是核心生成的插件密钥（名字 → 值）。
    pub fn resolve(
        manifest: &Manifest,
        generated: &BTreeMap<String, String>,
        langs: &Languages,
    ) -> Result<Option<Self>, String> {
        let Some(proxy) = manifest
            .webui
            .as_ref()
            .filter(|w| w.sso == Sso::Proxy)
            .and_then(|w| w.proxy.as_ref())
        else {
            return Ok(None);
        };
        let resolve = |template: &str| -> Result<Vec<Part>, String> {
            parse(template, langs)?
                .into_iter()
                .map(|part| match part {
                    Part::Secret(name) => generated
                        .get(&name)
                        .map(|value| Part::Secret(value.clone()))
                        .ok_or_else(|| secret_missing(langs, &name)),
                    other => Ok(other),
                })
                .collect()
        };
        let headers = proxy
            .headers
            .iter()
            .map(|(name, template)| {
                let name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| e.to_string())?;
                Ok((name, resolve(template)?))
            })
            .collect::<Result<_, String>>()?;
        let cookies = proxy
            .cookies
            .iter()
            .map(|(name, template)| Ok((name.clone(), resolve(template)?)))
            .collect::<Result<_, String>>()?;
        Ok(Some(Self { headers, cookies }))
    }

    /// 代加的请求头：名字和这次请求的值。
    pub fn headers(
        &self,
        owner: &Owner,
        plugin_admin: bool,
    ) -> Result<Vec<(HeaderName, String)>, RenderError> {
        let role = role_name(plugin_admin);
        self.headers
            .iter()
            .map(|(name, parts)| Ok((name.clone(), render(parts, owner, Some(role))?)))
            .collect()
    }

    /// 代加的 Cookie：名字和这次请求的值。
    pub fn cookies(
        &self,
        owner: &Owner,
        plugin_admin: bool,
    ) -> Result<Vec<(&str, String)>, RenderError> {
        let role = role_name(plugin_admin);
        self.cookies
            .iter()
            .map(|(name, parts)| Ok((name.as_str(), render(parts, owner, Some(role))?)))
            .collect()
    }

    /// 代加的 Cookie 的名字。
    pub fn cookie_names(&self) -> impl Iterator<Item = &str> {
        self.cookies.iter().map(|(name, _)| name.as_str())
    }

    /// 代加的请求头的名字。
    pub fn header_names(&self) -> impl Iterator<Item = &HeaderName> {
        self.headers.iter().map(|(name, _)| name)
    }

    /// 模板里用到的密钥的值：不能出现在给浏览器的响应里。
    pub fn secrets(&self) -> Vec<&str> {
        let header_parts = self.headers.iter().map(|(_, parts)| parts);
        let cookie_parts = self.cookies.iter().map(|(_, parts)| parts);
        header_parts
            .chain(cookie_parts)
            .flatten()
            .filter_map(|part| match part {
                Part::Secret(value) => Some(value.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Render `clients.http.auth.gateway.headers` with the same placeholders as
/// `webui.proxy.headers`.
pub fn render_gateway_headers(
    headers: &BTreeMap<String, String>,
    owner: &Owner,
    plugin_admin: bool,
    generated: &BTreeMap<String, String>,
    langs: &Languages,
) -> Result<Vec<(HeaderName, String)>, String> {
    headers
        .iter()
        .map(|(name, template)| {
            let name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|error| error.to_string())?;
            let parts = parse(template, langs)?
                .into_iter()
                .map(|part| match part {
                    Part::Secret(name) => generated
                        .get(&name)
                        .map(|value| Part::Secret(value.clone()))
                        .ok_or_else(|| secret_missing(langs, &name)),
                    other => Ok(other),
                })
                .collect::<Result<Vec<_>, String>>()?;
            let value = render(&parts, owner, Some(role_name(plugin_admin)))
                .map_err(|error| error.to_string())?;
            Ok((name, value))
        })
        .collect()
}

/// 模板要了不认识请求者角色时，不生成代登录凭据；身份资料本身都已经能取到。
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("unknown proxy identity role")]
    UnknownRole,
}

fn role_name(plugin_admin: bool) -> &'static str {
    if plugin_admin { "admin" } else { "user" }
}

fn render(parts: &[Part], owner: &Owner, role: Option<&str>) -> Result<String, RenderError> {
    let mut value = String::new();
    let mut email = None;
    for part in parts {
        value.push_str(match part {
            Part::Text(text) | Part::Secret(text) => text,
            Part::User => &owner.username,
            Part::Name => &owner.display_name,
            Part::Role => role.ok_or(RenderError::UnknownRole)?,
            Part::Email => email.get_or_insert_with(|| internal_email(&owner.username)),
        });
    }
    Ok(value)
}

/// LAN 账号模板：没有请求者角色，拒绝 `{role}`；密钥必须由核心生成且足够长。
pub fn check_template(
    manifest: &Manifest,
    template: &str,
    langs: &Languages,
) -> Result<(), String> {
    for part in parse(template, langs)? {
        match part {
            Part::Role => return Err(role_without_request(langs, template)),
            Part::Secret(name) if !secret_ok(manifest, &name) => {
                return Err(unknown_secret(langs, &name));
            }
            _ => {}
        }
    }
    Ok(())
}

/// 按主人和插件密钥填好 LAN 账号模板；不把网页请求的角色上下文带进 LAN。
pub fn fill_template(
    template: &str,
    owner: &Owner,
    generated: &BTreeMap<String, String>,
    langs: &Languages,
) -> Result<String, String> {
    let parts = parse(template, langs)?
        .into_iter()
        .map(|part| match part {
            Part::Role => Err(role_without_request(langs, template)),
            Part::Secret(name) => generated
                .get(&name)
                .map(|value| Part::Secret(value.clone()))
                .ok_or_else(|| secret_missing(langs, &name)),
            other => Ok(other),
        })
        .collect::<Result<Vec<_>, String>>()?;
    render(&parts, owner, None).map_err(|e| e.to_string())
}

fn role_without_request(langs: &Languages, template: &str) -> String {
    text(
        langs,
        "plugin-install-proxy-template-unknown-var",
        &[("template", template.into()), ("var", "role".into())],
    )
}
