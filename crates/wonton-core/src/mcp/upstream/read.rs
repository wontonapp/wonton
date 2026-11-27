use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{ReadResourceRequestParams, ResourceContents};
use tokio::time::Instant;

use super::Upstreams;
use super::error::{CallFailure, ReadError};
use crate::plugin::McpServer;

impl Upstreams {
    /// 读插件的资源（`resources/read`），不超过 `limit` 字节。结果在内存里，也经过一次 JSON：只适合小文件，大文件插件要给内网
    /// 地址。
    pub async fn read_resource(
        &self,
        server: &McpServer,
        uri: &str,
        limit: usize,
    ) -> Result<Vec<u8>, ReadError> {
        let result = self.read_contents(server, uri).await?;
        let Some(contents) = result.contents.into_iter().next() else {
            return Err(ReadError::Upstream(format!(
                "插件 {} 的资源 {uri} 是空的",
                server.id
            )));
        };
        let bytes = match contents {
            ResourceContents::TextResourceContents { text, .. } => text.into_bytes(),
            ResourceContents::BlobResourceContents { blob, .. } => {
                STANDARD.decode(blob).map_err(|e| {
                    ReadError::Upstream(format!(
                        "插件 {} 的资源 {uri} 不是合法的 base64：{e}",
                        server.id
                    ))
                })?
            }
            other => {
                return Err(ReadError::Upstream(format!(
                    "插件 {} 的资源 {uri} 是读不懂的类型：{other:?}",
                    server.id
                )));
            }
        };
        if bytes.len() > limit {
            return Err(ReadError::TooLarge);
        }
        Ok(bytes)
    }

    /// 读插件的资源，连同内容的 `_meta`（MCP Apps 界面要按它设 CSP）。大小上限同上。
    pub async fn read_contents(
        &self,
        server: &McpServer,
        uri: &str,
    ) -> Result<rmcp::model::ReadResourceResult, ReadError> {
        let connection = self
            .connection(server)
            .await
            .map_err(ReadError::Unreachable)?;
        let id = &server.id;
        let params = ReadResourceRequestParams::new(uri.to_owned());
        let started = Instant::now();
        let _running = connection.calling(started);
        let read = connection.peer().read_resource(params);
        let result = match tokio::time::timeout(self.read_timeout, read).await {
            Ok(Ok(result)) => {
                self.answered(id, &connection);
                result
            }
            Ok(Err(e)) => {
                return Err(match self.failure(id, &connection, e) {
                    CallFailure::TooLarge(_) => ReadError::TooLarge,
                    CallFailure::Replaced => {
                        ReadError::Unreachable(format!("插件 {id} 的会话不回应，已被换掉"))
                    }
                    CallFailure::Transport(e) => {
                        ReadError::Unreachable(format!("插件 {id} 读不了资源 {uri}：{e}"))
                    }
                    CallFailure::Plugin(e) => {
                        ReadError::Upstream(format!("插件 {id} 读不了资源 {uri}：{e}"))
                    }
                });
            }
            Err(_) => {
                self.timed_out(id, &connection, started);
                return Err(ReadError::Unreachable(format!(
                    "插件 {id} 的资源 {uri} 在 {:?} 内没有读完",
                    self.read_timeout
                )));
            }
        };
        Ok(result)
    }
}
