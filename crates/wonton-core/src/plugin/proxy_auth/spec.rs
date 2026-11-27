use hyper::header::{HeaderName, HeaderValue};

use super::validate::control_chars_error;
use super::{MCP_PROTOCOL_HEADERS, part_name};
use crate::l10n::text;
use crate::plugin::languages::Languages;

/// 不能代加的请求头：Host 和 Cookie 由网关自己处理，转发头由网关重新设置，逐跳头不能转发，
/// 能力头只能由网关按这个人注入（页面自己带的会被删掉，清单也不能声明）。
pub(super) const FORBIDDEN_HEADERS: &[&str] = &[
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
    "x-wonton-capabilities",
];

/// 值模板里的一段。`Secret` 解析清单时是密钥的名字，网关用的时候是它的值。
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Part {
    Text(String),
    User,
    Name,
    Email,
    Role,
    Secret(String),
}

/// 远端 MCP 的请求头（`remote_mcp.headers`）：名字合法、不在禁止名单里（`X-Wonton-*` 由核心写，MCP 的协议头由
/// 传输层写），值是合法头值。返回模板引用的密钥名，只有 `{secret:…}` 能用。
pub fn validate_header_spec(
    name: &str,
    template: &str,
    langs: &Languages,
) -> Result<Vec<String>, String> {
    let lower = name.to_ascii_lowercase();
    if HeaderName::from_bytes(name.as_bytes()).is_err()
        || FORBIDDEN_HEADERS.contains(&lower.as_str())
        || lower.starts_with("x-forwarded-")
        || lower.starts_with("x-wonton-")
        || MCP_PROTOCOL_HEADERS.contains(&lower.as_str())
    {
        return Err(text(
            langs,
            "plugin-install-remote-header-forbidden",
            &[("name", name.into())],
        ));
    }
    let mut secrets = Vec::new();
    for part in parse(template, langs)? {
        match part {
            Part::Secret(secret) => secrets.push(secret),
            Part::Text(text) if HeaderValue::from_str(&text).is_err() => {
                return Err(control_chars_error(langs, template));
            }
            Part::Text(_) => {}
            other => {
                return Err(text(
                    langs,
                    "plugin-install-remote-identity-placeholder",
                    &[
                        ("placeholder", part_name(&other).into()),
                        ("template", template.into()),
                    ],
                ));
            }
        }
    }
    Ok(secrets)
}

/// 拆开一个值模板。`{`、`}` 只能用来写占位符。
pub(super) fn parse(template: &str, langs: &Languages) -> Result<Vec<Part>, String> {
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
            "role" => Part::Role,
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
pub(super) fn is_cookie_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?={}".contains(&b))
}

/// RFC 6265 的 cookie-octet：可见 ASCII，除去 `"`、`,`、`;` 和 `\`。生成的密钥（十六进制、Base64）都满足。
pub(super) fn is_cookie_value(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_graphic() && !b"\",;\\".contains(&b))
}
