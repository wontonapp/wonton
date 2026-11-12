//! 媒体：把消息里的媒体来源编成不透明的句柄，凭句柄下载原文件。
//!
//! 加密房间里的附件，来源里带着解密密钥，所以句柄也要和密钥一样保管，不能写进日志。

use matrix_sdk::{
    media::{MediaFormat, MediaRequestParameters},
    ruma::{OwnedMxcUri, events::room::MediaSource},
};

use crate::{Error, ImageContent, MatrixSession, Result};

/// 把未加密的 Matrix 媒体地址编成可交给 [`MatrixSession::download_media`] 的句柄。
pub fn media_token(url: &str) -> Result<String> {
    let uri: OwnedMxcUri = url.into();
    uri.validate()
        .map_err(|error| Error::InvalidInput(format!("Matrix 媒体地址无效：{error}")))?;
    serde_json::to_string(&MediaSource::Plain(uri))
        .map_err(|error| Error::InvalidInput(format!("媒体句柄编码失败：{error}")))
}

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
    /// 用当前登录账号上传媒体，返回服务器分配的 `mxc://` 地址。
    pub async fn upload_media(&self, mime_type: &str, data: Vec<u8>) -> Result<String> {
        let content_type: mime::Mime = mime_type
            .parse()
            .map_err(|error| Error::InvalidInput(format!("Invalid media MIME type: {error}")))?;
        let response = self
            .client
            .media()
            .upload(&content_type, data, None)
            .await?;
        Ok(response.content_uri.to_string())
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_handle_rejects_non_mxc_and_incomplete_urls() {
        for url in ["https://example.org/avatar", "mxc://example.org", "mxc://"] {
            assert!(matches!(media_token(url), Err(Error::InvalidInput(_))));
        }
    }
}
