//! 文件服务要核心别的部分做的事：找插件的服务、读插件的资源、认房间里的 Bot、给主人发消息。

use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::json;
use wonton_protocol::FileFailure;

use super::resource::InternalUrl;
use super::slots::{PluginSlots, SLOT_WAIT};
use super::{Failure, MAX_RESOURCE_BYTES};
use crate::l10n;
use crate::matrix::{MatrixError, NoticeRoom};
use crate::mcp::upstream::{ReadError, Upstreams};
use crate::plugin::languages::Languages;
use crate::plugin::upstreams::ResourceUpstreamError;
use crate::plugin::{McpServer, PluginId, PluginManager};

pub trait FileHost: Send + Sync {
    /// 来源插件在运行，否则 `plugin_unavailable`。
    fn check_plugin(&self, plugin: &PluginId) -> Result<(), Failure>;
    /// 来源插件在运行，也提供 MCP 服务，否则 `plugin_unavailable`。
    fn check_mcp(&self, plugin: &PluginId) -> Result<(), Failure>;
    /// 核心访问这个内网地址要连的 `host:port`。主机名必须是来源插件 compose 里的服务名，否则 `permission_denied`。
    fn service_upstream<'a>(
        &'a self,
        plugin: &'a PluginId,
        url: &'a InternalUrl,
    ) -> BoxFuture<'a, Result<String, Failure>>;
    /// `resources/read`，最多 [`MAX_RESOURCE_BYTES`]。
    fn read_resource<'a>(
        &'a self,
        plugin: &'a PluginId,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, Failure>>;
    /// 插件现在有没有这个共享文件夹的授权，读或写都算。
    fn share_granted(&self, plugin: &PluginId, share: &str) -> Result<bool, Failure>;
    /// 这个房间里的 Bot 的名字。
    fn bots_in_room(&self, room: &str) -> Result<Vec<String>, Failure>;
    /// 在和主人的“Wonton”私聊里发一条文字。
    fn notify_owner<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<(), Failure>>;
}

/// 真正的实现：插件管理器、插件的 MCP 服务和通知房间。
pub struct CoreHost {
    plugins: Arc<PluginManager>,
    upstreams: Arc<Upstreams>,
    /// 读到的资源都在内存里，每个最多 [`MAX_RESOURCE_BYTES`]。
    reads: PluginSlots,
    /// 没接 Matrix 时为空。
    notices: Option<Arc<NoticeRoom>>,
}

impl CoreHost {
    pub fn new(
        plugins: Arc<PluginManager>,
        upstreams: Arc<Upstreams>,
        notices: Option<Arc<NoticeRoom>>,
    ) -> Self {
        Self {
            plugins,
            upstreams,
            reads: PluginSlots::new(),
            notices,
        }
    }

    fn mcp_server(&self, plugin: &PluginId) -> Result<McpServer, Failure> {
        self.check_plugin(plugin)?;
        let langs = self.plugins.owner_languages();
        self.plugins
            .mcp_servers()
            .into_iter()
            .find(|server| &server.id == plugin)
            .ok_or_else(|| {
                Failure::new(
                    FileFailure::PluginUnavailable,
                    l10n::text(
                        &langs,
                        "files-mcp-gone",
                        &[("plugin", plugin.as_str().into())],
                    ),
                )
            })
    }
}

fn unavailable(plugin: &PluginId, langs: &Languages) -> Failure {
    Failure::new(
        FileFailure::PluginUnavailable,
        l10n::text(
            langs,
            "files-plugin-unavailable",
            &[("plugin", plugin.as_str().into())],
        ),
    )
}

impl FileHost for CoreHost {
    fn check_plugin(&self, plugin: &PluginId) -> Result<(), Failure> {
        self.plugins
            .is_running(plugin)
            .then_some(())
            .ok_or_else(|| unavailable(plugin, &self.plugins.owner_languages()))
    }

    fn check_mcp(&self, plugin: &PluginId) -> Result<(), Failure> {
        self.mcp_server(plugin).map(|_| ())
    }

    fn service_upstream<'a>(
        &'a self,
        plugin: &'a PluginId,
        url: &'a InternalUrl,
    ) -> BoxFuture<'a, Result<String, Failure>> {
        async move {
            let langs = self.plugins.owner_languages();
            self.plugins
                .resource_upstream(plugin, &url.service, url.port)
                .await
                .map_err(|e| upstream_failure(plugin, &url.service, e, &langs))
        }
        .boxed()
    }

    fn read_resource<'a>(
        &'a self,
        plugin: &'a PluginId,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, Failure>> {
        async move {
            let server = self.mcp_server(plugin)?;
            let langs = self.plugins.owner_languages();
            let _slot = self.reads.acquire(plugin, SLOT_WAIT, &langs).await?;
            self.upstreams
                .read_resource(&server, uri, MAX_RESOURCE_BYTES)
                .await
                .map_err(read_failure)
        }
        .boxed()
    }

    fn share_granted(&self, plugin: &PluginId, share: &str) -> Result<bool, Failure> {
        self.plugins
            .share_granted(plugin, share)
            .map_err(|e| Failure::internal(e.to_string()))
    }

    fn bots_in_room(&self, room: &str) -> Result<Vec<String>, Failure> {
        self.plugins
            .bots_in_room(room)
            .map_err(|e| Failure::internal(e.to_string()))
    }

    fn notify_owner<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        async move {
            let langs = self.plugins.owner_languages();
            let notices = self.notices.as_ref().ok_or_else(|| {
                Failure::new(
                    FileFailure::ServiceUnavailable,
                    l10n::text(&langs, "files-notify-no-matrix", &[]),
                )
            })?;
            let content = json!({"msgtype": "m.text", "body": text});
            notices
                .send(&content)
                .await
                .map(|_| ())
                .map_err(|e| notice_failure(e, &langs))
        }
        .boxed()
    }
}

fn upstream_failure(
    plugin: &PluginId,
    service: &str,
    error: ResourceUpstreamError,
    langs: &Languages,
) -> Failure {
    match error {
        ResourceUpstreamError::Unavailable => unavailable(plugin, langs),
        ResourceUpstreamError::NotItsService => Failure::new(
            FileFailure::PermissionDenied,
            l10n::text(
                langs,
                "files-upstream-not-own",
                &[
                    ("service", service.into()),
                    ("plugin", plugin.as_str().into()),
                ],
            ),
        ),
        ResourceUpstreamError::Internal(why) => Failure::internal(why),
    }
}

fn read_failure(error: ReadError) -> Failure {
    let kind = match error {
        ReadError::Unreachable(_) => FileFailure::PluginUnavailable,
        ReadError::Upstream(_) => FileFailure::UpstreamError,
        ReadError::TooLarge => FileFailure::ResourceTooLarge,
    };
    Failure::new(kind, error.to_string())
}

/// Tuwunel 连不上、自己出了问题、让核心慢一点（`M_LIMIT_EXCEEDED`）才算服务不可用，等一等会好。它拒绝了请求（通知房间已经换过
/// 一次，还是不行）等多久都不会好，是核心这边的问题。
fn notice_failure(error: MatrixError, langs: &Languages) -> Failure {
    let unavailable = match &error {
        MatrixError::Unavailable { .. } => true,
        MatrixError::Rejected { status, .. } => *status >= 500 || *status == 429,
        MatrixError::Invalid(_) | MatrixError::Storage(_) => false,
    };
    let why = l10n::text(
        langs,
        "files-matrix-send-failed",
        &[("error", error.to_string().into())],
    );
    if unavailable {
        Failure::new(FileFailure::ServiceUnavailable, why)
    } else {
        Failure::internal(why)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_unreachable_or_broken_matrix_server_is_unavailable() {
        let langs = Languages::from_header(None);
        let rejected = |status| MatrixError::Rejected {
            context: "发消息".into(),
            status,
            errcode: "M_FORBIDDEN".into(),
            message: "不在房间里".into(),
        };
        let kind = |error| notice_failure(error, &langs).kind;
        let unreachable = MatrixError::Unavailable {
            context: "发消息".into(),
            message: "连不上".into(),
        };
        assert_eq!(kind(unreachable), FileFailure::ServiceUnavailable);
        assert_eq!(kind(rejected(502)), FileFailure::ServiceUnavailable);
        assert_eq!(kind(rejected(429)), FileFailure::ServiceUnavailable);
        assert_eq!(kind(rejected(403)), FileFailure::Internal);
        assert_eq!(
            kind(MatrixError::Storage("写不了".into())),
            FileFailure::Internal
        );
    }
}
