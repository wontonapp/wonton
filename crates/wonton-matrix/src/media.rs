//! 媒体：把消息里的媒体来源编成不透明的句柄，凭句柄下载原文件。
//!
//! 加密房间里的附件，来源里带着解密密钥，所以句柄也要和密钥一样保管，不能写进日志。

use matrix_sdk::{
    media::{MediaFormat, MediaRequestParameters},
    ruma::{OwnedMxcUri, events::room::MediaSource},
};

use crate::{ChatRoom, Error, FileContent, ImageContent, MatrixSession, Result};

/// 链接预览的图片上传到媒体库后的来源（ADR-0065）：加密房间是加密文件，未加密只有 mxc 地址。
/// `Encrypted` 里的 JSON 原样进事件（`beeper:image:encryption`，MSC4095 的裸 EncryptedFile 形态）；
/// 壳侧要当媒体句柄用时外面还要包一层 `"file"`（ruma 的 `MediaSource` 反序列化认外层键），见
/// `wonton-client` 的 `previews::bundled_image`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewImageSource {
    /// 未加密：事件里写 `og:image`，值是 mxc 地址。
    Plain { mxc: String },
    /// 加密：事件里写 `beeper:image:encryption`，值是 EncryptedFile 的 JSON。
    Encrypted { file: serde_json::Value },
}

/// 把未加密的 Matrix 媒体地址编成可交给 [`MatrixSession::download_media`] 的句柄。
pub fn media_token(url: &str) -> Result<String> {
    let uri: OwnedMxcUri = url.into();
    uri.validate()
        .map_err(|error| Error::InvalidInput(format!("Matrix 媒体地址无效：{error}")))?;
    serde_json::to_string(&MediaSource::Plain(uri))
        .map_err(|error| Error::InvalidInput(format!("媒体句柄编码失败：{error}")))
}

/// 把事件里裸的 EncryptedFile JSON（MSC4095 的 `beeper:image:encryption`）编成可交给
/// [`MatrixSession::download_media`] 的句柄。`MediaSource::Encrypted` 反序列化认的是外面一层
/// `"file"`：不包一层，这个对象会被当成未加密的 mxc，取回来就是没解密的密文。
pub fn encrypted_media_token(file: &serde_json::Value) -> Result<String> {
    let source: MediaSource = serde_json::from_value(serde_json::json!({ "file": file }))
        .map_err(|error| Error::InvalidInput(format!("加密文件信息无效：{error}")))?;
    Ok(serde_json::to_string(&source).expect("媒体来源总能编成 JSON"))
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

impl FileContent {
    /// 取这个文件用的句柄，交给 [`MatrixSession::download_media`]。
    pub fn media_token(&self) -> String {
        serde_json::to_string(&self.source).expect("媒体来源总能编成 JSON")
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
        Ok(self
            .client
            .media()
            .get_media_content(&media_request(token)?, true)
            .await?)
    }
}

impl ChatRoom {
    /// 凭媒体句柄下载原文件，加密附件在本机解密；规则同 [`MatrixSession::download_media`]。
    pub async fn download_media(&self, token: &str) -> Result<Vec<u8>> {
        Ok(self
            .room
            .client()
            .media()
            .get_media_content(&media_request(token)?, true)
            .await?)
    }
}

/// 把句柄还原成媒体请求（[`MediaFormat::File`]，即原图原文件，加密的带密钥）。
fn media_request(token: &str) -> Result<MediaRequestParameters> {
    let source: MediaSource = serde_json::from_str(token)
        .map_err(|error| Error::InvalidInput(format!("媒体句柄无效：{error}")))?;
    Ok(MediaRequestParameters {
        source,
        format: MediaFormat::File,
    })
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
