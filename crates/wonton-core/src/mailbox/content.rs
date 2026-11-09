//! Mailpit v1.33.0 的内容契约；来源只认 Mailpit 加在最前面的 Received。

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

pub(super) const BODY_LIMIT: usize = 4000;
pub(super) const ATTACHMENT_LIMIT: u64 = 20 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct Message {
    #[serde(rename = "ID")]
    pub(super) id: String,
    pub(super) from: Option<Address>,
    #[serde(default)]
    pub(super) subject: String,
    #[serde(default)]
    pub(super) text: String,
    #[serde(default, rename = "HTML")]
    pub(super) html: String,
    #[serde(default, deserialize_with = "null_vec")]
    pub(super) to: Vec<Address>,
    #[serde(default, deserialize_with = "null_vec")]
    pub(super) cc: Vec<Address>,
    #[serde(default, deserialize_with = "null_vec")]
    pub(super) bcc: Vec<Address>,
    #[serde(default, deserialize_with = "null_vec")]
    pub(super) attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "null_vec")]
    pub(super) inline: Vec<Attachment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct Address {
    pub(super) address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct Attachment {
    #[serde(rename = "PartID")]
    pub(super) part_id: String,
    pub(super) file_name: String,
    pub(super) content_type: String,
    pub(super) size: u64,
}

fn null_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum ReceivedError {
    #[error("Mailpit message has no top Received header")]
    Missing,
    #[error("Mailpit top Received header has no valid remote IP")]
    Invalid,
}

pub(super) fn received_ip(headers: &Value) -> Result<IpAddr, ReceivedError> {
    let top = headers
        .get("Received")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .ok_or(ReceivedError::Missing)?;
    let top = top.as_str().ok_or(ReceivedError::Invalid)?;
    // EHLO 可带伪造的方括号甚至 “) by ”；真正的 remote IP 总在最后一个 “) by ” 之前。
    let (source, _) = top.rsplit_once(") by ").ok_or(ReceivedError::Invalid)?;
    let source = source
        .strip_prefix("from ")
        .and_then(|value| value.strip_suffix(']'))
        .ok_or(ReceivedError::Invalid)?;
    let (_, remote) = source.rsplit_once('[').ok_or(ReceivedError::Invalid)?;
    remote.parse().map_err(|_| ReceivedError::Invalid)
}

#[derive(Debug)]
pub(super) struct Prepared<'a> {
    pub(super) body: String,
    /// 超长正文对应的原始 HTML；调用者以 .html 文件上传，不当作聊天 HTML 渲染。
    pub(super) html_attachment: Option<&'a str>,
    pub(super) attachments: Vec<PreparedAttachment<'a>>,
}

#[derive(Debug)]
pub(super) struct PreparedAttachment<'a> {
    pub(super) attachment: &'a Attachment,
    pub(super) upload: bool,
}

impl Message {
    /// Mailpit 会把信封里有、To/Cc/Bcc 里没有的收件人补进 Bcc；因此取三个 API 字段的并集。
    /// 头里额外列出的现有用户也会收到；只使用调用者提供的现有内部地址，不按 From 判断身份。
    pub(super) fn recipients<'a>(&self, existing: &'a BTreeMap<String, String>) -> Vec<&'a str> {
        self.to
            .iter()
            .chain(&self.cc)
            .chain(&self.bcc)
            .filter_map(|address| existing.get(&address.address).map(String::as_str))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(super) fn prepare(&self) -> Result<Prepared<'_>, html2text::Error> {
        // 空白也是实际的 text 内容，不能擅自改选 HTML。
        let body = if !self.text.is_empty() {
            Cow::Borrowed(self.text.as_str())
        } else if !self.html.is_empty() {
            Cow::Owned(
                html2text::config::plain()
                    .no_link_wrapping()
                    .link_footnotes(true)
                    .string_from_read(self.html.as_bytes(), BODY_LIMIT)?,
            )
        } else {
            Cow::Borrowed("")
        };
        let cutoff = body.char_indices().nth(BODY_LIMIT).map(|(byte, _)| byte);
        let body = match body {
            Cow::Borrowed(text) => text[..cutoff.unwrap_or(text.len())].to_owned(),
            Cow::Owned(mut text) => {
                if let Some(byte) = cutoff {
                    text.truncate(byte);
                }
                text
            }
        };
        Ok(Prepared {
            body,
            html_attachment: (cutoff.is_some() && !self.html.is_empty())
                .then_some(self.html.as_str()),
            attachments: self
                .attachments
                .iter()
                .chain(&self.inline)
                .map(|attachment| PreparedAttachment {
                    attachment,
                    upload: attachment.size <= ATTACHMENT_LIMIT,
                })
                .collect(),
        })
    }
}

#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
