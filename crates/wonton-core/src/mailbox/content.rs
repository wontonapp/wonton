//! Mailpit v1.33.0 的内容契约；来源只认 Mailpit 加在最前面的 Received。

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

pub(super) const BODY_LIMIT: usize = 4000;
pub(super) const ATTACHMENT_LIMIT: u64 = 20 * 1024 * 1024;
/// 交给 html2text 的 HTML 上限（字符数）。转换在阻塞线程里跑，封顶保证一份几 MB 的 HTML 不会长时间占住线程；
/// 整封原始 HTML 仍作为 `message.html` 附件，转换只看到一段时也照样附带。
pub(super) const HTML_INPUT_LIMIT: usize = 1024 * 1024;
/// 转不出来时替给收件人的正文（原始 HTML 在附件里）。
pub(super) const CONVERT_FAILED_BODY: &str =
    "这封信是 HTML，正文转换失败；请用附件 message.html 查看原文。";

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

/// 交给收件人（或后台转换）的正文来源。
pub(super) enum BodySource<'a> {
    Text(&'a str),
    /// 只有 HTML：`html` 已按 [`HTML_INPUT_LIMIT`] 截断，`truncated` 表示只给了前一段。
    Html {
        html: String,
        truncated: bool,
    },
    Empty,
}

/// 转换后的正文；`attach_html` 表示整封原始 HTML 要作为附件（转换只看到一段或正文超长时）。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct Body {
    pub(super) text: String,
    pub(super) attach_html: bool,
}

#[cfg(test)]
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

    /// 正文来源：有 text 就用 text（空白也是实际的 text，不擅自改选 HTML），否则给截断后的 HTML。
    pub(super) fn body_source(&self) -> BodySource<'_> {
        if !self.text.is_empty() {
            return BodySource::Text(&self.text);
        }
        if self.html.is_empty() {
            return BodySource::Empty;
        }
        let (html, truncated) = truncate_chars(&self.html, HTML_INPUT_LIMIT);
        BodySource::Html { html, truncated }
    }

    /// 正文裁到 [`BODY_LIMIT`]；转换只看到 HTML 前一段或正文超长时，附整封原始 HTML。
    pub(super) fn finish_body(&self, text: &str, html_truncated: bool) -> Body {
        let (text, body_truncated) = truncate_chars(text, BODY_LIMIT);
        Body {
            text,
            attach_html: !self.html.is_empty() && (html_truncated || body_truncated),
        }
    }

    pub(super) fn attachments(&self) -> Vec<PreparedAttachment<'_>> {
        self.attachments
            .iter()
            .chain(&self.inline)
            .map(|attachment| PreparedAttachment {
                attachment,
                upload: attachment.size <= ATTACHMENT_LIMIT,
            })
            .collect()
    }

    /// 同步准备（只在测试里用；投递走缓存和阻塞线程）：转换出错就 `?` 抛出，不走进附件兜底。
    #[cfg(test)]
    pub(super) fn prepare(&self) -> Result<Prepared<'_>, html2text::Error> {
        let (body, attach_html) = match self.body_source() {
            BodySource::Text(text) => {
                let body = self.finish_body(text, false);
                (body.text, body.attach_html)
            }
            BodySource::Empty => (String::new(), false),
            BodySource::Html { html, truncated } => {
                let body = self.finish_body(&convert(&html)?, truncated);
                (body.text, body.attach_html)
            }
        };
        Ok(Prepared {
            body,
            html_attachment: (attach_html && !self.html.is_empty()).then_some(self.html.as_str()),
            attachments: self.attachments(),
        })
    }
}

/// 把 HTML 转成纯文本。调用方负责限长（见 [`HtmlInputLimit`](HTML_INPUT_LIMIT)）。
pub(super) fn convert(html: &str) -> Result<String, html2text::Error> {
    html2text::config::plain()
        .no_link_wrapping()
        .link_footnotes(true)
        .string_from_read(html.as_bytes(), BODY_LIMIT)
}

/// 按字符边界截到最多 `limit` 字节；返回是否截过。
fn truncate_chars(text: &str, limit: usize) -> (String, bool) {
    match text.char_indices().nth(limit) {
        Some((byte, _)) => (text[..byte].to_owned(), true),
        None => (text.to_owned(), false),
    }
}

#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
