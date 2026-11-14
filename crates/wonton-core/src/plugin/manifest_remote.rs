//! `runtime: remote_mcp` 的清单字段（ADR-0066 第 1、3 节）：远端服务器的地址、鉴权方式、请求头模板和工具范围。

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError};

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

/// 形态和字段对得上，`url`、鉴权方式和请求头模板合法。
pub(super) fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    super::remote_validate::validate(manifest, langs)
}
