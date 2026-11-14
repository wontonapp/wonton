//! `runtime: remote_mcp` 的清单字段（ADR-0066 第 1、3 节）：远端服务器的地址、鉴权方式、请求头模板和工具范围。

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError, Runtime};
use super::manifest_mcp;
use super::proxy_auth;
use super::secret_spec::SecretSpec;

pub use wonton_protocol::manifest::{RemoteAuth, RemoteMcpSpec};

/// 确认页要写的远端服务器（主机名和鉴权方式）；不是远端形态时为空。
pub(crate) fn preview(m: &Manifest) -> Option<wonton_protocol::RemoteMcpPreview> {
    m.remote_mcp
        .as_ref()
        .map(|remote| wonton_protocol::RemoteMcpPreview {
            host: remote.host().to_owned(),
            auth: remote.protocol_auth(),
        })
}

/// 详情里的远端服务器和授权状态。状态由连接器记，核心还没有问它的路径，要授权的先如实写“要授权”。
pub(crate) fn detail(m: &Manifest) -> Option<wonton_protocol::RemoteMcpDetail> {
    m.remote_mcp
        .as_ref()
        .map(|remote| wonton_protocol::RemoteMcpDetail {
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
        let secrets = proxy_auth::validate_header_spec(name, template, langs).map_err(invalid)?;
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
