//! `runtime: remote_mcp` 的清单字段（ADR-0066 第 1、3 节）：远端服务器的地址、鉴权方式、请求头模板和工具范围。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError, Runtime};
use super::manifest_mcp;
use super::proxy_auth;
use super::secret_spec::SecretSpec;

/// `x-wonton.remote_mcp`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteMcpSpec {
    /// 远端 MCP 服务器的 Streamable HTTP 地址。
    pub url: String,
    #[serde(default)]
    pub auth: RemoteAuth,
    /// `auth: header` 时核心发往提供方的每个请求都带上的头：名字 → 值模板，只能引用 `secrets` 里的 `input` 项。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// 只接这一份工具（上游的原名），同 `mcp.tools`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_idle_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_total_seconds: Option<u64>,
}

/// 远端服务器怎么认我们。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAuth {
    #[default]
    None,
    /// 请求头带主人填的密钥（ADR-0064）。
    Header,
    /// OAuth：App 拿授权码交给连接器，令牌只在连接器里（ADR-0066 第 3a 节）。
    Oauth,
}

impl RemoteMcpSpec {
    /// 主机名（带端口）：确认页和详情页写“数据发到 <主机>”。
    pub fn host(&self) -> &str {
        self.url
            .strip_prefix("https://")
            .unwrap_or(&self.url)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
    }

    /// 给 App 的鉴权方式。
    pub fn protocol_auth(&self) -> wonton_protocol::RemoteAuth {
        match self.auth {
            RemoteAuth::None => wonton_protocol::RemoteAuth::None,
            RemoteAuth::Header => wonton_protocol::RemoteAuth::Header,
            RemoteAuth::Oauth => wonton_protocol::RemoteAuth::Oauth,
        }
    }
}

/// 确认页要写的远端服务器（主机名和鉴权方式）；不是远端形态时为空。
pub(crate) fn preview(m: &Manifest) -> Option<wonton_protocol::RemoteMcpPreview> {
    m.remote_mcp.as_ref().map(|remote| wonton_protocol::RemoteMcpPreview {
        host: remote.host().to_owned(),
        auth: remote.protocol_auth(),
    })
}

/// 详情里的远端服务器和授权状态。状态由连接器记，核心还没有问它的路径，要授权的先如实写“要授权”。
pub(crate) fn detail(m: &Manifest) -> Option<wonton_protocol::RemoteMcpDetail> {
    m.remote_mcp.as_ref().map(|remote| wonton_protocol::RemoteMcpDetail {
        host: remote.host().to_owned(),
        auth: remote.protocol_auth(),
        authorization: match remote.auth {
            RemoteAuth::Oauth => wonton_protocol::RemoteAuthorization::NeedsAuthorization,
            _ => wonton_protocol::RemoteAuthorization::NotNeeded,
        },
    })
}

fn invalid(message: String) -> ManifestError {
    ManifestError::Invalid(message)
}

/// 形态和字段对得上，`url`、鉴权方式和请求头模板合法。
pub(super) fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let Some(remote) = &manifest.remote_mcp else {
        if manifest.runtime == Runtime::RemoteMcp {
            return Err(invalid(
                "runtime: remote_mcp 必须写 remote_mcp.url".to_owned(),
            ));
        }
        return Ok(());
    };
    if manifest.runtime != Runtime::RemoteMcp {
        return Err(invalid(
            "remote_mcp 只能和 runtime: remote_mcp 一起写".to_owned(),
        ));
    }
    check_url(&remote.url)?;
    match (remote.auth, remote.headers.is_empty()) {
        (RemoteAuth::Header, true) => {
            return Err(invalid(
                "remote_mcp.auth: header 还要写 remote_mcp.headers，不然请求里没有凭据".to_owned(),
            ));
        }
        (RemoteAuth::Header, false) => {}
        (auth, false) => {
            return Err(invalid(format!(
                "remote_mcp.headers 只在 auth: header 时写，现在是 {}",
                match auth {
                    RemoteAuth::None => "none",
                    RemoteAuth::Oauth => "oauth",
                    RemoteAuth::Header => unreachable!(),
                }
            )));
        }
        (_, true) => {}
    }
    for (name, template) in &remote.headers {
        let secrets = proxy_auth::validate_header_spec(name, template, langs)
            .map_err(invalid)?;
        if let Some(secret) = secrets
            .iter()
            .find(|secret| !matches!(manifest.secrets.get(*secret), Some(SecretSpec::Input(_))))
        {
            return Err(invalid(format!(
                "remote_mcp.headers.{name} may only reference `input` secrets, but {secret} is not one"
            )));
        }
    }
    manifest_mcp::validate_limits(
        "remote_mcp",
        remote.tool_idle_seconds,
        remote.tool_total_seconds,
        remote.tools.as_deref(),
    )
}

/// `url` 只收 `https`，不能带用户名密码和参数之外的空白，不能带片段。
fn check_url(url: &str) -> Result<(), ManifestError> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(invalid("remote_mcp.url 只能是 https 地址".to_owned()));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(invalid("remote_mcp.url 少了主机名".to_owned()));
    }
    if authority.contains('@') {
        return Err(invalid(
            "remote_mcp.url 不能带用户名和密码，凭据写在 secrets 里".to_owned(),
        ));
    }
    if url.chars().any(char::is_whitespace) {
        return Err(invalid("remote_mcp.url 不能有空白".to_owned()));
    }
    if let Some((_, fragment)) = url.split_once('#')
        && !fragment.is_empty()
    {
        return Err(invalid("remote_mcp.url 不能带 #片段".to_owned()));
    }
    Ok(())
}

