//! 装前握手与卸载收尾只访问能力提供方，核心不外连远端 MCP。

use std::time::Duration;

use hyper::{Method, StatusCode};
use serde::Deserialize;
use serde_json::Value;

use super::install_error::InstallError;
use super::languages::Languages;
use super::manager::PluginManager;
use super::manifest::{Manifest, PluginId};
use super::manifest_remote::RemoteAuth;
use super::remote_config::Config;
use super::secret_spec::SecretSpec;
use super::secrets::Secrets;
use super::upstreams::Endpoint;
use crate::l10n;

const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct ProbeResult {
    outcome: String,
    detail: String,
    #[serde(default)]
    http_status: Option<u16>,
}

/// OAuth 的授权码结构能交进来，但核心还没有交换和保存令牌的入口。
pub(super) fn check_oauth_unsupported(
    manifest: &Manifest,
    langs: &Languages,
) -> Result<(), InstallError> {
    if manifest
        .remote_mcp
        .as_ref()
        .is_some_and(|remote| remote.auth == RemoteAuth::Oauth)
    {
        return Err(InstallError::Unsupported(l10n::text(
            langs,
            "remote-install-oauth-unsupported",
            &[],
        )));
    }
    Ok(())
}

impl PluginManager {
    pub(super) async fn probe_remote(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let Some(remote) = &manifest.remote_mcp else {
            return Ok(());
        };
        check_oauth_unsupported(manifest, langs)?;
        let endpoint = self
            .hosted_provider("remote_mcp")
            .and_then(|provider| provider.endpoint)
            .ok_or_else(|| {
                InstallError::UpstreamUnavailable(l10n::text(
                    langs,
                    "remote-install-connector-unavailable",
                    &[],
                ))
            })?;
        let secrets = self.store.secrets().load(&manifest.id)?.unwrap_or_default();
        // ADR-0064：装好再填。凭据还没交给核心时无从握手，先装，填齐后下一次请求生效。
        if unresolved_input(manifest, &secrets, langs) {
            tracing::info!(plugin = %manifest.id, "远端 MCP 的密钥还没填齐，跳过装前握手");
            return Ok(());
        }
        let config = Config::resolve(manifest, &secrets, langs).map_err(InstallError::Invalid)?;
        let request = serde_json::to_value(&config).expect("字符串配置可序列化");
        let response = self
            .connector_json(&endpoint, Method::POST, "/_wonton/probe", Some(&request))
            .await;
        let (status, value) = response.map_err(|_| connector_failed(langs))?;
        if !status.is_success() {
            return Err(connector_failed(langs));
        }
        let result: ProbeResult =
            serde_json::from_value(value).map_err(|_| connector_failed(langs))?;
        if matches!(result.outcome.as_str(), "ok" | "legacy_sse") {
            return Ok(());
        }
        let key = match result.outcome.as_str() {
            "needs_oauth" => "remote-install-needs-oauth",
            "needs_header_secret" => "remote-install-needs-header",
            "secret_rejected" => "remote-install-secret-rejected",
            "not_mcp" => "remote-install-not-mcp",
            "blocked_address" => "remote-install-blocked-address",
            "unreachable" => "remote-install-unreachable",
            "proxy_unavailable" => "remote-install-proxy-unavailable",
            "remote_error" => "remote-install-remote-error",
            _ => return Err(connector_failed(langs)),
        };
        let detail = config.redact(&result.detail);
        let reason = l10n::text(langs, key, &[]);
        let message = l10n::text(
            langs,
            "remote-install-probe-failed",
            &[
                ("reason", reason.into()),
                ("url", remote.url.as_str().into()),
                ("detail", detail.into()),
            ],
        );
        tracing::warn!(plugin = %manifest.id, url = %remote.url, outcome = %result.outcome,
            http_status = result.http_status, "Remote MCP probe rejected installation");
        Err(match result.outcome.as_str() {
            "unreachable" => InstallError::RemoteUnavailable(message),
            "remote_error" => InstallError::RemoteError(message),
            "proxy_unavailable" => InstallError::EgressProxyUnavailable {
                reason: wonton_protocol::EgressFailure::ProxyUnreachable,
                message,
            },
            "secret_rejected" => InstallError::RemoteInvalid {
                message,
                secrets: remote
                    .headers
                    .values()
                    .filter_map(|template| {
                        super::proxy_auth::validate_input_template(manifest, template, langs).ok()
                    })
                    .flatten()
                    .chain(
                        super::proxy_auth::validate_input_template(manifest, &remote.url, langs)
                            .unwrap_or_default(),
                    )
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            },
            _ => InstallError::Invalid(message),
        })
    }

    /// 卸载状态已撤下总线。清理失败只进入卸载报告，不阻止删本机记录。
    pub(super) async fn forget_remote(&self, id: &PluginId, langs: &Languages) -> Option<String> {
        let remote = self
            .plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| listed.manifest.remote_mcp.is_some());
        if !remote {
            return None;
        }
        self.forget_connector(id, langs).await
    }

    /// 失败安装的记录已经不在插件表里，调用方按清单判断过后直接调这里。
    /// 连接器对已删插件回 404 算删过：重试卸载不该报成清理失败。
    pub(super) async fn forget_connector(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Option<String> {
        let endpoint = self
            .hosted_provider("remote_mcp")
            .and_then(|provider| provider.endpoint);
        let result = match endpoint {
            Some(endpoint) => self
                .connector_json(
                    &endpoint,
                    Method::DELETE,
                    &format!("/_wonton/plugins/{id}"),
                    None,
                )
                .await
                .map(|(status, _)| status.is_success() || status == StatusCode::NOT_FOUND),
            None => Err(()),
        };
        if result != Ok(true) {
            tracing::warn!(plugin = %id, "Remote MCP connector token cleanup failed");
            Some(l10n::text(langs, "remote-install-forget-failed", &[]))
        } else {
            None
        }
    }

    async fn connector_json(
        &self,
        endpoint: &Endpoint,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(StatusCode, Value), ()> {
        let Some(socket) = endpoint.socket() else {
            return self
                .http
                .json(method, &endpoint.url(path), body, TIMEOUT)
                .await
                .map_err(|_| ());
        };
        let client = reqwest::Client::builder()
            .unix_socket(socket.to_path_buf())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ())?;
        let mut request = client.request(method, endpoint.url(path));
        if let Some(body) = body {
            request = request.json(body);
        }
        let exchange = async {
            let response = request.send().await.map_err(|_| ())?;
            let status = response.status();
            let bytes = axum::body::to_bytes(
                axum::body::Body::from_stream(response.bytes_stream()),
                1 << 20,
            )
            .await
            .map_err(|_| ())?;
            let value = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).map_err(|_| ())?
            };
            Ok((status, value))
        };
        tokio::time::timeout(TIMEOUT, exchange)
            .await
            .map_err(|_| ())?
    }
}

/// 地址或必填请求头还引用着没填的密钥：此刻握不了手，先让插件装上等主人填。
pub(super) fn unresolved_input(manifest: &Manifest, secrets: &Secrets, langs: &Languages) -> bool {
    let Some(remote) = &manifest.remote_mcp else {
        return false;
    };
    let referenced = |template: &String| {
        super::proxy_auth::validate_input_template(manifest, template, langs).unwrap_or_default()
    };
    // 地址拼不出来就总是算没填齐，和必填与否无关。
    if referenced(&remote.url)
        .into_iter()
        .any(|name| !secrets.contains_key(&name))
    {
        return true;
    }
    remote
        .headers
        .values()
        .flat_map(referenced)
        .any(|name| missing_required(manifest, secrets, &name))
}

fn missing_required(manifest: &Manifest, secrets: &Secrets, name: &str) -> bool {
    !secrets.contains_key(name)
        && matches!(
            manifest.secrets.get(name),
            Some(SecretSpec::Input(input)) if input.required
        )
}

fn connector_failed(langs: &Languages) -> InstallError {
    InstallError::UpstreamUnavailable(l10n::text(
        langs,
        "remote-install-connector-unavailable",
        &[],
    ))
}
