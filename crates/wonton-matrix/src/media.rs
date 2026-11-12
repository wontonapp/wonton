//! 媒体：把消息里的媒体来源编成不透明的句柄，凭句柄下载原文件。
//!
//! 加密房间里的附件，来源里带着解密密钥，所以句柄也要和密钥一样保管，不能写进日志。

use matrix_sdk::{
    media::{MediaFormat, MediaRequestParameters},
    ruma::events::room::MediaSource,
};

use crate::{Error, ImageContent, MatrixSession, Result};

impl ImageContent {
    /// 取这张图片原图用的句柄，交给 [`MatrixSession::download_media`]。
    pub fn media_token(&self) -> String {
        serde_json::to_string(&self.source).expect("媒体来源总能编成 JSON")
    }

    /// 发送端给的缩略图的句柄；没有缩略图时为 `None`。
    pub fn thumbnail_token(&self) -> Option<String> {
        self.thumbnail
            .as_ref()
            .map(|source| serde_json::to_string(source).expect("媒体来源总能编成 JSON"))
    }
}

impl MatrixSession {
    /// 凭 [`ImageContent::media_token`] 给出的句柄下载原文件。加密附件在本机解密；下载过的文件存在本地
    /// 存储里，再取不用重新下载。
    pub async fn download_media(&self, token: &str) -> Result<Vec<u8>> {
        let source: MediaSource = serde_json::from_str(token)
            .map_err(|error| Error::InvalidInput(format!("媒体句柄无效：{error}")))?;
        let request = MediaRequestParameters {
            source,
            format: MediaFormat::File,
        };
        Ok(self
            .client
            .media()
            .get_media_content(&request, true)
            .await?)
    }
}
