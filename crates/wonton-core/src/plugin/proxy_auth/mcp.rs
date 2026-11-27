use std::collections::{BTreeSet, HashMap};

use hyper::header::{HeaderName, HeaderValue};

use super::spec::{FORBIDDEN_HEADERS, Part, parse};
use super::validate::control_chars_error;
use crate::l10n::text;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::{Manifest, ManifestError, SecretSpec};
use crate::plugin::secrets::Secrets;

/// MCP 上游自己的协议头：rmcp 按会话和协商的版本设置它们，清单声明了也带不上去。
pub(super) const MCP_PROTOCOL_HEADERS: &[&str] = &["mcp-session-id", "mcp-protocol-version"];

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

/// 同上，但头里引用了还没填的用户填写密钥时不算错：头先空着，插件照样装，必填的由总线按未就绪拒绝并点名
/// （ADR-0064）；填齐后重建容器，这时再解析。其余的解析错误照旧上报：引用到的 `collect` 项没值，
/// 对端服务已经跑完却说不出为什么，不能跟着用户的输入一起埋掉。
pub fn resolve_mcp_headers_or_pending(
    manifest: &Manifest,
    secrets: &Secrets,
    langs: &Languages,
) -> Result<McpHeaders, String> {
    let resolved = resolve_mcp_headers(manifest, secrets, langs);
    if resolved.is_err() {
        let missing = missing_refs(manifest, secrets, langs);
        if !missing.is_empty() && missing.iter().all(|(_, input)| *input) {
            return Ok(McpHeaders::default());
        }
    }
    resolved
}

/// `mcp.headers` 里引用到、还没有值的密钥，带“是不是用户填写项”。模板写法已经在清单校验里查过，
/// 这里解析不出来就跳过。
fn missing_refs(manifest: &Manifest, secrets: &Secrets, langs: &Languages) -> Vec<(String, bool)> {
    let Some(mcp) = &manifest.mcp else {
        return Vec::new();
    };
    mcp.headers
        .values()
        .filter_map(|template| parse(template, langs).ok())
        .flatten()
        .filter_map(|part| match part {
            Part::Secret(name) if !secrets.contains_key(&name) => {
                let input = matches!(manifest.secrets.get(&name), Some(SecretSpec::Input(_)));
                Some((name, input))
            }
            _ => None,
        })
        .collect()
}

/// `mcp.headers` 引用的密钥名。这些项安装/升级时必须已经有值（规范“MCP 调用”节），其余 `collect` 项缺了
/// 可以等核心下次核对再收（“插件生成、核心保管”节）。
pub fn mcp_header_secret_refs(manifest: &Manifest, langs: &Languages) -> BTreeSet<String> {
    manifest
        .mcp
        .as_ref()
        .map(|mcp| {
            mcp.headers
                .values()
                .filter_map(|template| parse(template, langs).ok())
                .flatten()
                .filter_map(|part| match part {
                    Part::Secret(name) => Some(name),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
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
                Part::User | Part::Name | Part::Email | Part::Role => {
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

pub(super) fn part_name(part: &Part) -> &'static str {
    match part {
        Part::User => "user",
        Part::Name => "name",
        Part::Email => "email",
        Part::Role => "role",
        Part::Text(_) | Part::Secret(_) => "",
    }
}

/// 按插件密钥把 `mcp.headers` 的模板填好。`collect` 的值是文件原样：不是文本、含换行或控制字符（结尾的
/// 换行也算，`echo` 写的文件就带）、或者密钥还没有值，都报出到头和密钥，不带值本身。
///
/// 必填的用户填写密钥还没填时另算：见 [`resolve_mcp_headers_or_pending`]。
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
                Part::User | Part::Name | Part::Email | Part::Role => {
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
