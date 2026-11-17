//! 清单的 `clients`：第三方原生 App 从局域网或公网连进来的入口（ADR-0021、ADR-0089、ADR-0090）。
//!
//! 解析在 `wonton_protocol::manifest`，校验在这里。运行时怎么兑现（入口容器、开关、注入 `WONTON_EXTERNAL_URL`）在别的 PR 里。
//! 服务名是否存在由 [`super::package`] 对照 compose 检查。

use std::collections::BTreeSet;

use serde_json::Value;

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError};
use crate::l10n;

/// 路径匹配的条数上限（`public.allow`、`public.deny`）。
const MAX_PATHS: usize = 64;
/// 一条路径的字节上限。
const MAX_PATH_BYTES: usize = 256;
/// `clients.tcp[].name` 的字节上限。
const MAX_TCP_NAME_BYTES: usize = 32;

pub use wonton_protocol::manifest::{ClientsHttp, ClientsTcp, CredentialsAuth};

/// `clients` 和 `network.clients` 要一起写；一次报全，不逐条停下来。
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let clients = manifest.clients.as_ref();
    let permitted = manifest
        .permissions
        .contains(&super::manifest::Permission::NetworkClients);
    let mut problems = Vec::new();
    let guests = manifest.webui.as_ref().is_some_and(|webui| webui.guests);
    match (clients, permitted) {
        (None, false) => {}
        // 只声明能接待匿名用户也算有入口，入口上只放拿着有效访客链接的请求。
        (None, true) if guests => {}
        (None, true) => problems.push(l10n::text(
            langs,
            "plugin-settings-clients-need-clients",
            &[],
        )),
        (Some(_), false) => problems.push(l10n::text(
            langs,
            "plugin-settings-clients-need-permission",
            &[],
        )),
        (Some(clients), true) => {
            if clients.http.is_none() && clients.tcp.as_ref().is_none_or(Vec::is_empty) {
                problems.push(l10n::text(langs, "plugin-settings-clients-empty", &[]));
            }
            if let Some(http) = &clients.http {
                validate_http(manifest, http, langs, &mut problems);
            }
            if let Some(tcp) = &clients.tcp {
                validate_tcp(manifest, tcp, langs, &mut problems);
            }
        }
    }
    // 访客链接也建在入口上（ADR-0090），只声明能接待匿名用户、不开权限时开不出入口。
    if guests && !permitted {
        problems.push(l10n::text(langs, "plugin-settings-clients-guests", &[]));
    }
    finish(problems)
}

/// 引用 `${WONTON_EXTERNAL_URL}` 的清单必须开得出入口：变量只在 `clients.http` 或 `webui.guests: true`
/// 时有值（ADR-0089 第 5 节）。
pub fn validate_external_url(
    manifest: &Manifest,
    referenced: bool,
    langs: &Languages,
) -> Result<(), ManifestError> {
    let served = manifest.clients.as_ref().is_some_and(|c| c.http.is_some())
        || manifest.webui.as_ref().is_some_and(|webui| webui.guests);
    let problems = (referenced && !served)
        .then(|| l10n::text(langs, "plugin-settings-clients-external-url", &[]))
        .into_iter()
        .collect();
    finish(problems)
}

fn validate_http(
    manifest: &Manifest,
    http: &ClientsHttp,
    langs: &Languages,
    problems: &mut Vec<String>,
) {
    let mcp = manifest.mcp.as_ref().map(|m| (&m.service, m.port));
    if http.port == 0 {
        problems.push(l10n::text(
            langs,
            "plugin-settings-clients-port-zero",
            &[("field", "http".into())],
        ));
    }
    if mcp == Some((&http.service, http.port)) {
        problems.push(port_mcp(langs, "http", &http.service, http.port));
    }
    let modes = [
        http.auth.gateway.is_some(),
        http.auth.credentials.is_some(),
        http.auth.app.is_some(),
    ];
    if modes.iter().filter(|written| **written).count() != 1 {
        problems.push(l10n::text(langs, "plugin-settings-clients-auth", &[]));
    }
    if let Some(gateway) = &http.auth.gateway {
        if let Err(problem) = super::proxy_auth::validate_headers(manifest, &gateway.headers, langs)
        {
            problems.push(problem.to_string());
        }
        if http
            .auth_failures
            .as_ref()
            .is_some_and(|list| !list.is_empty())
        {
            problems.push(l10n::text(langs, "plugin-settings-clients-gateway", &[]));
        }
    }
    if let Some(credentials) = &http.auth.credentials {
        validate_credentials(credentials, langs, problems);
    }
    if let Some(app) = &http.auth.app
        && app.setup_done.is_empty()
    {
        problems.push(l10n::text(langs, "plugin-settings-clients-setup-done", &[]));
    }
    if let Some(public) = &http.public {
        if let Some(allow) = &public.allow {
            validate_paths("public.allow", allow, langs, problems);
        }
        if let Some(deny) = &public.deny {
            validate_paths("public.deny", deny, langs, problems);
        }
        if public.allow.is_some() == public.deny.is_some() {
            problems.push(l10n::text(langs, "plugin-settings-clients-public", &[]));
        }
    }
    if let Some(failures) = &http.auth_failures {
        if failures.len() > MAX_PATHS {
            problems.push(l10n::text(
                langs,
                "plugin-settings-clients-paths",
                &[("field", "auth_failures".into())],
            ));
        }
        for failure in failures {
            if let Some(problem) = path_problem("auth_failures", &failure.path, langs) {
                problems.push(problem);
            }
            if failure.status.is_empty()
                || failure
                    .status
                    .iter()
                    .any(|status| !(400..=599).contains(status))
            {
                problems.push(l10n::text(
                    langs,
                    "plugin-settings-clients-status",
                    &[(
                        "status",
                        l10n::list(
                            langs,
                            &failure
                                .status
                                .iter()
                                .map(u16::to_string)
                                .collect::<Vec<_>>(),
                        )
                        .into(),
                    )],
                ));
            }
        }
    }
}

fn validate_credentials(
    credentials: &CredentialsAuth,
    langs: &Languages,
    problems: &mut Vec<String>,
) {
    if credentials.issue.is_empty() {
        problems.push(l10n::text(langs, "plugin-settings-clients-issue", &[]));
    }
    if credentials.per_client {
        if credentials.revoke.is_none() {
            problems.push(l10n::text(langs, "plugin-settings-clients-per-client", &[]));
        }
    } else if credentials.rotate.is_none() || credentials.revoke.is_some() {
        problems.push(l10n::text(langs, "plugin-settings-clients-shared", &[]));
    }
    if credentials.generates && credentials.verify.is_none() {
        problems.push(l10n::text(langs, "plugin-settings-clients-generates", &[]));
    }
}

fn validate_tcp(
    manifest: &Manifest,
    tcp: &[ClientsTcp],
    langs: &Languages,
    problems: &mut Vec<String>,
) {
    let webui = manifest.webui.as_ref().map(|w| (&w.service, w.port));
    let mcp = manifest.mcp.as_ref().map(|m| (&m.service, m.port));
    let mut names = BTreeSet::new();
    for entry in tcp {
        // SSO 的信任头伪造得了，所以 TCP 入口不能指向认它的 MCP 和网页端口。
        let port = (&entry.service, entry.port);
        if entry.port == 0 {
            problems.push(l10n::text(
                langs,
                "plugin-settings-clients-port-zero",
                &[("field", "tcp".into())],
            ));
        } else if webui == Some(port) {
            problems.push(l10n::text(
                langs,
                "plugin-settings-clients-tcp-webui",
                &[
                    ("service", entry.service.as_str().into()),
                    ("port", entry.port.to_string().into()),
                ],
            ));
        } else if mcp == Some(port) {
            problems.push(port_mcp(langs, "tcp", &entry.service, entry.port));
        }
        if !is_tcp_name(&entry.name) {
            problems.push(l10n::text(
                langs,
                "plugin-settings-clients-tcp-name",
                &[("name", entry.name.as_str().into())],
            ));
        } else if !names.insert(entry.name.as_str()) {
            problems.push(l10n::text(
                langs,
                "plugin-settings-clients-tcp-duplicate",
                &[("name", entry.name.as_str().into())],
            ));
        }
    }
}

fn validate_paths(field: &str, paths: &[String], langs: &Languages, problems: &mut Vec<String>) {
    if paths.len() > MAX_PATHS {
        problems.push(l10n::text(
            langs,
            "plugin-settings-clients-paths",
            &[("field", field.into())],
        ));
    }
    for path in paths {
        if let Some(problem) = path_problem(field, path, langs) {
            problems.push(problem);
        }
    }
}

/// 路径规则：以 `/` 开头，不含 `%`、`\`、`;`、`?`、`#`、空白和点段。解码几次、怎么切段各家不同，所以只收
/// 一种写法，让前缀匹配没有歧义（ADR-0021“路径怎么匹配”）。
fn path_problem(field: &str, path: &str, langs: &Languages) -> Option<String> {
    let bytes = path.len();
    let unsafe_char = path
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || "-._~!$&'()*+,=:@/".contains(ch)));
    let dotted = path
        .split('/')
        .any(|segment| segment == "." || segment == "..");
    (bytes > MAX_PATH_BYTES || !path.starts_with('/') || unsafe_char || dotted).then(|| {
        l10n::text(
            langs,
            "plugin-settings-clients-path",
            &[
                ("field", field.into()),
                ("path", path.into()),
                ("bytes", bytes.to_string().into()),
            ],
        )
    })
}

fn is_tcp_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_TCP_NAME_BYTES
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

fn port_mcp(langs: &Languages, field: &str, service: &str, port: u16) -> String {
    l10n::text(
        langs,
        "plugin-settings-clients-port-mcp",
        &[
            ("field", field.into()),
            ("service", service.into()),
            ("port", port.to_string().into()),
        ],
    )
}

/// 服务（compose 里每个服务的 JSON 值）里有没有引用 `WONTON_EXTERNAL_URL`。
pub(super) fn references_external_url(services: &serde_json::Map<String, Value>) -> bool {
    fn scan(value: &Value, found: &mut bool) {
        match value {
            Value::String(text) => {
                let _ = super::interpolate::references(text, &mut |name| {
                    *found |= name == "WONTON_EXTERNAL_URL";
                });
            }
            Value::Array(items) => items.iter().for_each(|item| scan(item, found)),
            Value::Object(fields) => fields.values().for_each(|item| scan(item, found)),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    let mut found = false;
    for value in services.values() {
        scan(value, &mut found);
    }
    found
}

fn finish(problems: Vec<String>) -> Result<(), ManifestError> {
    if problems.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::Invalid(problems.join("\n")))
    }
}

#[cfg(test)]
#[path = "manifest_clients_tests.rs"]
mod tests;
