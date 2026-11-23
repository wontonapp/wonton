//! 远端 MCP 每次使用密钥库里的值；展开的地址和凭据只进入连接器请求。

use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;

use super::entry::{Entry, PluginStatus};
use super::languages::Languages;
use super::manager::PluginManager;
use super::manifest::{Manifest, PluginId};
use super::manifest_remote::RemoteAuth;
use super::mcp_server::{McpServer, ToolLimits};
use super::proxy_auth::{self, McpHeaders};
use super::remote_redaction::Redaction;
use super::secrets::Secrets;
use crate::l10n;

// No Debug: both the URL and forwarded headers can contain input secrets.
#[derive(Serialize)]
pub(super) struct Config {
    pub url: String,
    pub auth: &'static str,
    pub headers: BTreeMap<String, String>,
    #[serde(skip)]
    pub redaction: Option<Redaction>,
}

impl Config {
    pub(super) fn resolve(
        manifest: &Manifest,
        secrets: &Secrets,
        langs: &Languages,
    ) -> Result<Self, String> {
        let remote = manifest.remote_mcp.as_ref().expect("远端形态已经校验");
        let url = proxy_auth::render_input_url(manifest, secrets, &remote.url, langs)?;
        let url = super::remote_validate::parse_expanded_url(&url, langs)?.to_string();
        let mut headers = BTreeMap::new();
        for (name, template) in &remote.headers {
            let Some(value) = proxy_auth::render_input_header(manifest, secrets, template, langs)?
            else {
                continue;
            };
            headers.insert(name.to_ascii_lowercase(), value);
        }
        let redaction = Redaction::new(
            url.as_str(),
            &remote.url,
            secrets
                .values()
                .filter_map(|value| std::str::from_utf8(value).ok()),
        );
        Ok(Self {
            url,
            auth: match remote.auth {
                RemoteAuth::None => "none",
                RemoteAuth::Header => "header",
                RemoteAuth::Oauth => "oauth",
            },
            headers,
            redaction,
        })
    }

    fn bus_headers(&self, id: &PluginId) -> Result<McpHeaders, String> {
        let forward = STANDARD.encode(serde_json::to_vec(&self.headers).expect("字符串表可序列化"));
        McpHeaders::from_sensitive_pairs([
            ("x-wonton-plugin", id.to_string()),
            ("x-wonton-remote-url", self.url.clone()),
            ("x-wonton-remote-auth", self.auth.to_owned()),
            ("x-wonton-remote-forward", forward),
        ])
        .map(|headers| headers.with_error_redaction(self.redaction.clone()))
    }

    /// 连接器的详情不可信，可能把请求地址或密钥原样带回来。
    pub(super) fn redact(&self, detail: &str) -> String {
        match &self.redaction {
            Some(redaction) => redaction.text(detail).into_owned(),
            None => detail.to_owned(),
        }
    }
}

/// 能接上总线的远端服务，以及没能解析出来的插件和原因（状态里显示异常）。
pub(super) fn servers(
    manager: &PluginManager,
    plugins: &BTreeMap<PluginId, Entry>,
) -> (Vec<McpServer>, Vec<(PluginId, String)>) {
    let langs = manager.owner_languages();
    let mut servers = Vec::new();
    let mut failures = Vec::new();
    for (id, entry) in plugins {
        let Some(manifest) = entry.listed.as_ref().map(|listed| &listed.manifest) else {
            continue;
        };
        let Some(remote) = manifest.remote_mcp.as_ref() else {
            continue;
        };
        // 停了的插件先让开，哪怕还留着上次的异常记录；运行中的往下走，改好了才能把记录清掉。
        if entry.status != PluginStatus::Running {
            continue;
        }
        let Some(endpoint) = super::install_hosted::provider_in(plugins, "remote_mcp", &langs)
            .and_then(|provider| provider.endpoint)
        else {
            continue;
        };
        let secrets = match manager.store.secrets().load(id) {
            Ok(secrets) => secrets.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(plugin = %id, url = %remote.url, %error, "Remote MCP secret store unreadable");
                failures.push((
                    id.clone(),
                    l10n::text(
                        &langs,
                        "remote-install-secrets-unreadable",
                        &[("error", error.to_string().into())],
                    ),
                ));
                continue;
            }
        };
        if super::remote_install::unresolved_input(manifest, &secrets, &langs) {
            failures.push((
                id.clone(),
                l10n::text(&langs, "remote-install-secret-missing", &[]),
            ));
            continue;
        }
        let headers = match Config::resolve(manifest, &secrets, &langs)
            .and_then(|config| config.bus_headers(id))
        {
            Ok(headers) => headers,
            Err(error) => {
                tracing::warn!(plugin = %id, url = %remote.url, %error, "Remote MCP configuration unavailable");
                failures.push((
                    id.clone(),
                    l10n::text(
                        &langs,
                        "remote-install-secrets-unreadable",
                        &[("error", error.into())],
                    ),
                ));
                continue;
            }
        };
        let defaults = ToolLimits::default();
        servers.push(McpServer {
            id: id.clone(),
            title: entry.title.clone(),
            url: endpoint.url("/mcp"),
            socket: endpoint.socket().map(std::path::Path::to_path_buf),
            headers,
            tool_limits: ToolLimits {
                idle: remote
                    .tool_idle_seconds
                    .map_or(defaults.idle, Duration::from_secs),
                total: remote
                    .tool_total_seconds
                    .map_or(defaults.total, Duration::from_secs),
            },
            tools: remote.tools.clone(),
        });
    }
    (servers, failures)
}
