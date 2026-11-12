//! 网关代登录（`sso: proxy`，插件规范的“网关代登录”）：清单声明的请求头和 Cookie 的值模板。网关认出主人后把它们
//! 加在转发给插件网页的请求上（见 `gateway::proxy_sso`），浏览器拿不到其中的密钥。
//!
//! 值模板里可以用 `{user}`、`{name}`、`{email}`（主人的用户名、显示名和邮箱），以及 `{secret:<名字>}`（插件自己的
//! `secrets` 里由核心生成的一项），其余文字原样保留。

use std::collections::{BTreeMap, HashMap};

use hyper::header::{HeaderName, HeaderValue};

use super::manifest::{Manifest, ManifestError, SecretSpec, Sso};
use super::secrets::Secrets;
use crate::owner::Owner;

use super::languages::Languages;
use crate::l10n::text;
use crate::owner::no_email;

/// 代登录用的密钥至少这么长：网关按值从响应里找出并去掉它，值太短会误伤别的内容。
const MIN_SECRET_BYTES: usize = 16;

/// 不能代加的请求头：Host 和 Cookie 由网关自己处理，转发头由网关重新设置，逐跳头不能转发。
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "cookie",
    "forwarded",
    "x-real-ip",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// 值模板里的一段。`Secret` 解析清单时是密钥的名字，网关用的时候是它的值。
#[derive(Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    User,
    Name,
    Email,
    Secret(String),
}

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
        langs: &Languages,
    ) -> Result<Vec<(HeaderName, String)>, NoEmail> {
        self.headers
            .iter()
            .map(|(name, parts)| Ok((name.clone(), render(parts, owner, langs)?)))
            .collect()
    }

    /// 代加的 Cookie：名字和这次请求的值。
    pub fn cookies(
        &self,
        owner: &Owner,
        langs: &Languages,
    ) -> Result<Vec<(&str, String)>, NoEmail> {
        self.cookies
            .iter()
            .map(|(name, parts)| Ok((name.as_str(), render(parts, owner, langs)?)))
            .collect()
    }

    /// 代加的 Cookie 的名字。
    pub fn cookie_names(&self) -> impl Iterator<Item = &str> {
        self.cookies.iter().map(|(name, _)| name.as_str())
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

/// 模板用了 `{email}`，主人却没填邮箱：不代加一个空值。消息按请求的语言写好（ADR-0043）。
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct NoEmail(pub String);

fn render(parts: &[Part], owner: &Owner, langs: &Languages) -> Result<String, NoEmail> {
    parts
        .iter()
        .map(|part| {
            Ok(match part {
                Part::Text(text) | Part::Secret(text) => text.as_str(),
                Part::User => owner.username.as_str(),
                Part::Name => owner.display_name.as_str(),
                Part::Email => owner
                    .email
                    .as_deref()
                    .ok_or_else(|| NoEmail(no_email(langs)))?,
            })
        })
        .collect()
}

/// 值模板的写法对，`{secret:…}` 只引用插件自己由核心生成、够长的密钥。局域网客户端的账号（`lan.client`）也用这套模板。
pub fn check_template(
    manifest: &Manifest,
    template: &str,
    langs: &Languages,
) -> Result<(), String> {
    for part in parse(template, langs)? {
        if let Part::Secret(name) = part
            && !secret_ok(manifest, &name)
        {
            return Err(unknown_secret(langs, &name));
        }
    }
    Ok(())
}

/// 按主人和插件密钥填好一个值模板。
pub fn fill_template(
    template: &str,
    owner: &Owner,
    generated: &BTreeMap<String, String>,
    langs: &Languages,
) -> Result<String, String> {
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
    render(&parts, owner, langs).map_err(|e| e.to_string())
}

fn secret_ok(manifest: &Manifest, name: &str) -> bool {
    matches!(
        manifest.secrets.get(name),
        Some(SecretSpec::Generate(g)) if g.bytes >= MIN_SECRET_BYTES
    )
}

fn unknown_secret(langs: &Languages, name: &str) -> String {
    text(
        langs,
        "plugin-install-proxy-secret-unknown",
        &[("name", name.into()), ("min", MIN_SECRET_BYTES.into())],
    )
}

fn secret_missing(langs: &Languages, name: &str) -> String {
    text(
        langs,
        "plugin-install-proxy-secret-missing",
        &[("name", name.into())],
    )
}

/// 清单的代登录模板用没用 `{email}`。
pub(super) fn uses_email(manifest: &Manifest) -> bool {
    any_part(manifest, |part| matches!(part, Part::Email))
}

/// 清单的代登录模板会不会把主人的身份带给应用。
pub fn uses_identity(manifest: &Manifest) -> bool {
    any_part(manifest, |part| {
        matches!(part, Part::User | Part::Name | Part::Email)
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

/// 拆开一个值模板。`{`、`}` 只能用来写占位符。
fn parse(template: &str, langs: &Languages) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find(['{', '}']) {
        if rest[start..].starts_with('}') {
            return Err(text(
                langs,
                "plugin-install-proxy-template-open",
                &[("template", template.into())],
            ));
        }
        if start > 0 {
            parts.push(Part::Text(rest[..start].to_owned()));
        }
        let after = &rest[start + 1..];
        let end = after.find('}').ok_or_else(|| {
            text(
                langs,
                "plugin-install-proxy-template-close",
                &[("template", template.into())],
            )
        })?;
        parts.push(match &after[..end] {
            "user" => Part::User,
            "name" => Part::Name,
            "email" => Part::Email,
            other => match other.strip_prefix("secret:") {
                Some(name) if !name.is_empty() => Part::Secret(name.to_owned()),
                _ => {
                    return Err(text(
                        langs,
                        "plugin-install-proxy-template-unknown-var",
                        &[("template", template.into()), ("var", other.into())],
                    ));
                }
            },
        });
        rest = &after[end + 1..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest.to_owned()));
    }
    Ok(parts)
}

/// RFC 6265 的 Cookie 名：token，不含空白和分隔符。
fn is_cookie_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?={}".contains(&b))
}

/// RFC 6265 的 cookie-octet：可见 ASCII，除去 `"`、`,`、`;` 和 `\`。生成的密钥（十六进制、Base64）都满足。
fn is_cookie_value(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_graphic() && !b"\",;\\".contains(&b))
}

fn invalid(message: String) -> ManifestError {
    ManifestError::Invalid(message)
}

/// 清单校验：请求头名和 Cookie 名合法；模板写法对，`{secret:…}` 只引用插件自己的 `secrets` 里由核心生成、
/// 至少 16 字节的项；Cookie 的值只能用 `{secret:…}` 和 cookie-octet 组成的固定文字（用户名、显示名和邮箱可能有
/// Cookie 里不能出现的字符）。
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let Some(proxy) = manifest.webui.as_ref().and_then(|w| w.proxy.as_ref()) else {
        return Ok(());
    };
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
    for (name, template) in &proxy.headers {
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
        check_template(template, false)?;
    }
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

fn cookie_value_error(langs: &Languages, template: &str) -> String {
    text(
        langs,
        "plugin-install-proxy-cookie-chars",
        &[("template", template.into())],
    )
}

fn control_chars_error(langs: &Languages, template: &str) -> String {
    text(
        langs,
        "plugin-install-proxy-header-control-chars",
        &[("template", template.into())],
    )
}

/// MCP 上游自己的协议头：rmcp 按会话和协商的版本设置它们，清单声明了也带不上去。
const MCP_PROTOCOL_HEADERS: &[&str] = &["mcp-session-id", "mcp-protocol-version"];

/// MCP 上游（`mcp.headers`）的请求头：名字和按插件密钥解析好的值。`Debug` 只打印名字，值不进日志。
#[derive(Clone, Default, PartialEq, Eq)]
pub struct McpHeaders(Vec<(HeaderName, HeaderValue)>);

impl std::fmt::Debug for McpHeaders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.0.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_tuple("McpHeaders").field(&names).finish()
    }
}

impl McpHeaders {
    /// rmcp 传输层的 `custom_headers`：发往上游的每个请求都带上。
    pub fn custom_headers(&self) -> HashMap<HeaderName, HeaderValue> {
        self.0.iter().cloned().collect()
    }

    #[cfg(test)]
    pub(crate) fn from_pairs(pairs: impl IntoIterator<Item = (&'static str, String)>) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(name, value)| {
                    (
                        HeaderName::from_static(name),
                        HeaderValue::from_str(&value).expect("测试里的头值要合法"),
                    )
                })
                .collect(),
        )
    }
}

fn mcp_invalid(message: String, langs: &Languages) -> ManifestError {
    ManifestError::Invalid(text(
        langs,
        "plugin-install-proxy-mcp-prefix",
        &[("error", message.into())],
    ))
}

/// 清单校验：`mcp.headers` 的头名合法且不在禁止名单里（MCP 的协议头也算）；模板写法对，`{secret:…}` 引用
/// 插件 `secrets` 里声明的项（generate 或 collect 都行，这里没有网关按值去密钥的需求，不限长度）；`{user}`
/// 这类身份占位符不能用——MCP 调用不是主人的浏览器请求，上游认的是应用签发给核心的凭据。
pub fn validate_mcp_headers(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let Some(mcp) = &manifest.mcp else {
        return Ok(());
    };
    for (name, template) in &mcp.headers {
        let lower = name.to_ascii_lowercase();
        if HeaderName::from_bytes(name.as_bytes()).is_err()
            || FORBIDDEN_HEADERS.contains(&lower.as_str())
            || lower.starts_with("x-forwarded-")
            || MCP_PROTOCOL_HEADERS.contains(&lower.as_str())
        {
            return Err(mcp_invalid(
                text(
                    langs,
                    "plugin-install-proxy-mcp-header-forbidden",
                    &[("name", name.into())],
                ),
                langs,
            ));
        }
        for part in parse(template, langs).map_err(|e| mcp_invalid(e, langs))? {
            match part {
                Part::Secret(secret) if !manifest.secrets.contains_key(&secret) => {
                    return Err(mcp_invalid(
                        text(
                            langs,
                            "plugin-install-proxy-mcp-secret-undeclared",
                            &[("name", secret.into())],
                        ),
                        langs,
                    ));
                }
                Part::User | Part::Name | Part::Email => {
                    return Err(mcp_invalid(
                        text(
                            langs,
                            "plugin-install-proxy-mcp-identity-placeholder",
                            &[
                                ("placeholder", part_name(&part).into()),
                                ("template", template.into()),
                            ],
                        ),
                        langs,
                    ));
                }
                Part::Text(text) if HeaderValue::from_str(&text).is_err() => {
                    return Err(mcp_invalid(control_chars_error(langs, template), langs));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn part_name(part: &Part) -> &'static str {
    match part {
        Part::User => "user",
        Part::Name => "name",
        Part::Email => "email",
        Part::Text(_) | Part::Secret(_) => "",
    }
}

/// 按插件密钥把 `mcp.headers` 的模板填好。`collect` 的值是文件原样：不是文本、含换行或控制字符（结尾的
/// 换行也算，`echo` 写的文件就带）、或者密钥还没有值，都报出到头和密钥，不带值本身。
pub fn resolve_mcp_headers(
    manifest: &Manifest,
    secrets: &Secrets,
    langs: &Languages,
) -> Result<McpHeaders, String> {
    let Some(mcp) = &manifest.mcp else {
        return Ok(McpHeaders::default());
    };
    let mut headers = Vec::new();
    for (name, template) in &mcp.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
            text(
                langs,
                "plugin-install-proxy-mcp-bad-name",
                &[("name", name.into()), ("error", e.to_string().into())],
            )
        })?;
        let mut value = String::new();
        for part in parse(template, langs)? {
            match part {
                Part::Text(text) => value.push_str(&text),
                Part::Secret(secret) => match secrets.get(&secret) {
                    Some(bytes) => {
                        let value_text = String::from_utf8(bytes.clone()).map_err(|_| {
                            text(
                                langs,
                                "plugin-install-proxy-secret-not-text",
                                &[
                                    ("name", name.as_str().into()),
                                    ("secret", secret.as_str().into()),
                                ],
                            )
                        })?;
                        if HeaderValue::from_str(&value_text).is_err() {
                            return Err(text(
                                langs,
                                "plugin-install-proxy-secret-newline",
                                &[
                                    ("name", name.as_str().into()),
                                    ("secret", secret.as_str().into()),
                                ],
                            ));
                        }
                        value.push_str(&value_text);
                    }
                    None => {
                        return Err(text(
                            langs,
                            "plugin-install-proxy-secret-no-value",
                            &[
                                ("secret", secret.as_str().into()),
                                ("name", name.as_str().into()),
                            ],
                        ));
                    }
                },
                // validate_mcp_headers 已经拒绝了身份占位符，到不了这里。
                Part::User | Part::Name | Part::Email => {
                    return Err(text(
                        langs,
                        "plugin-install-proxy-mcp-no-identity",
                        &[("name", name.as_str().into())],
                    ));
                }
            }
        }
        // 值不进错误消息：它可能是密钥。标记 sensitive，传输层的调试输出不打印它。
        let mut value = HeaderValue::from_str(&value).map_err(|_| {
            text(
                langs,
                "plugin-install-proxy-mcp-bad-value",
                &[("name", name.as_str().into())],
            )
        })?;
        value.set_sensitive(true);
        headers.push((name, value));
    }
    Ok(McpHeaders(headers))
}
