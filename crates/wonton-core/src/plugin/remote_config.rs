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
use super::proxy_auth::{self, InputSecretEncoding, McpHeaders};
use super::remote_redaction::Redaction;
use super::secrets::Secrets;

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
        let url = proxy_auth::render_input_template(
            manifest,
            secrets,
            &remote.url,
            InputSecretEncoding::UrlComponent,
            langs,
        )?;
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

pub(super) fn servers<'a>(
    manager: &'a PluginManager,
    plugins: &'a BTreeMap<PluginId, Entry>,
) -> impl Iterator<Item = McpServer> + 'a {
    let langs = manager.owner_languages();
    plugins.iter().filter_map(move |(id, entry)| {
        let manifest = &entry.listed.as_ref()?.manifest;
        let remote = manifest.remote_mcp.as_ref()?;
        if super::install_hosted::effective_status(plugins, entry, &langs) != PluginStatus::Running {
            return None;
        }
        let provider = super::install_hosted::provider_in(plugins, "remote_mcp", &langs)?;
        let endpoint = provider.endpoint?;
        let headers = (|| {
            let secrets = manager.store.secrets().load(id).map_err(|error| error.to_string())?
                .unwrap_or_default();
            Config::resolve(manifest, &secrets, &langs)?.bus_headers(id)
        })();
        let headers = match headers {
            Ok(headers) => headers,
            Err(error) => {
                tracing::warn!(plugin = %id, url = %remote.url, %error, "Remote MCP configuration unavailable");
                return None;
            }
        };
        let defaults = ToolLimits::default();
        Some(McpServer {
            id: id.clone(),
            title: entry.title.clone(),
            url: endpoint.url("/mcp"),
            socket: endpoint.socket().map(std::path::Path::to_path_buf),
            headers,
            tool_limits: ToolLimits {
                idle: remote.tool_idle_seconds.map_or(defaults.idle, Duration::from_secs),
                total: remote.tool_total_seconds.map_or(defaults.total, Duration::from_secs),
            },
            tools: remote.tools.clone(),
        })
    })
}
