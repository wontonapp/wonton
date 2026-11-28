//! 在“Wonton”私聊里提醒主人新请求。

use std::sync::Arc;

use futures_util::future::BoxFuture;

use super::{PeerRequest, Peers};

/// 在“Wonton”私聊里提醒主人（生产里是 `matrix::NoticeRoom`）。发不出去只记日志，不影响配对。
pub trait OwnerNotices: Send + Sync + 'static {
    /// 发一条消息，内容是 Matrix 的消息体（`m.text`）。
    fn send(&self, content: serde_json::Value) -> BoxFuture<'_, Result<(), String>>;
}

impl OwnerNotices for crate::matrix::NoticeRoom {
    fn send(&self, content: serde_json::Value) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.send(&content)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    }
}

impl Peers {
    /// 在“Wonton”私聊里提醒主人有一条新请求，带跳到“其他服务器”的链接。
    pub(in crate::peers) fn notify(self: &Arc<Self>, request: &PeerRequest) {
        let Some(notices) = self.notices.clone() else {
            return;
        };
        let langs = (self.languages)();
        let who = if request.owner_display_name.trim().is_empty() {
            request.server_name.clone()
        } else {
            request.owner_display_name.clone()
        };
        let text = format!(
            "{}\n{}",
            crate::l10n::text(
                &langs,
                "peers-new-request",
                &[
                    ("owner", who.into()),
                    ("server", request.server_name.as_str().into()),
                ],
            ),
            wonton_protocol::AppLink::Peers.to_url()
        );
        let server_name = request.server_name.clone();
        tokio::spawn(async move {
            let content = serde_json::json!({"msgtype": "m.text", "body": text});
            if let Err(e) = notices.send(content).await {
                tracing::info!(server = %server_name, error = %e, "配对提醒没有发进私聊");
            }
        });
    }
}
