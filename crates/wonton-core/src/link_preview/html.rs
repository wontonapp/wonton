//! 从网页开头找预览要的东西：Open Graph、Twitter Card、`<title>` 和 `<meta name=description>`，只看 `<head>`。
//!
//! 只用 html5ever 的分词器，不建 DOM：要的都是 `<head>` 里的几个标签，分词器已经处理好实体、属性和 `<script>`
//! 里的假标签。

use std::cell::RefCell;

use encoding_rs::{Encoding, UTF_8};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, CharacterTokens, EndTag, StartTag, Tag, TagToken, Token, TokenSink,
    TokenSinkResult, Tokenizer, TokenizerOpts,
};

/// 标题、摘要截到这么多字：卡片上显示不了更多。
const MAX_TITLE_CHARS: usize = 300;
const MAX_DESCRIPTION_CHARS: usize = 1000;
/// 按 HTML 规范，`<meta charset>` 只在前 1024 字节里找。
const CHARSET_PRESCAN: usize = 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Head {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    /// `og:image` 的原值，可能是相对地址。
    pub image: Option<String>,
}

impl Head {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.description.is_none() && self.image.is_none()
    }
}

/// `charset` 来自 `Content-Type`；没有时在开头找 `<meta charset>`，再没有按 UTF-8（带 BOM 的按 BOM）。
pub(super) fn parse(bytes: &[u8], charset: Option<&str>) -> Head {
    let encoding = charset
        .and_then(|label| Encoding::for_label(label.trim().as_bytes()))
        .or_else(|| prescan(&bytes[..bytes.len().min(CHARSET_PRESCAN)]))
        .unwrap_or(UTF_8);
    let (text, _, _) = encoding.decode(bytes);
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(text.as_ref()));
    let tokenizer = Tokenizer::new(Sink::default(), TokenizerOpts::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.found.into_inner().finish()
}

/// `charset=` 后面的编码名，`<meta charset>` 和 `http-equiv` 两种写法都认。只是粗找，找错了解码出乱码，不会出别的事。
fn prescan(head: &[u8]) -> Option<&'static Encoding> {
    let lower = head.to_ascii_lowercase();
    let at = lower.windows(8).position(|w| w == b"charset=")? + 8;
    let rest = &lower[at..];
    let rest = rest
        .strip_prefix(b"\"")
        .or(rest.strip_prefix(b"'"))
        .unwrap_or(rest);
    let end = rest
        .iter()
        .position(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.')))
        .unwrap_or(rest.len());
    Encoding::for_label(&rest[..end])
}

#[derive(Default)]
struct Found {
    og_title: Option<String>,
    twitter_title: Option<String>,
    title: Option<String>,
    og_description: Option<String>,
    twitter_description: Option<String>,
    description: Option<String>,
    site_name: Option<String>,
    image: Option<String>,
    /// 正在读 `<title>` 的文字。
    in_title: Option<String>,
    /// 到了 `</head>` 或 `<body>`，后面的不看。
    done: bool,
}

impl Found {
    fn finish(self) -> Head {
        Head {
            title: self
                .og_title
                .or(self.twitter_title)
                .or(self.title)
                .and_then(|t| clean(&t, MAX_TITLE_CHARS)),
            description: self
                .og_description
                .or(self.twitter_description)
                .or(self.description)
                .and_then(|t| clean(&t, MAX_DESCRIPTION_CHARS)),
            site_name: self.site_name.and_then(|t| clean(&t, MAX_TITLE_CHARS)),
            image: self
                .image
                .map(|t| t.trim().to_owned())
                .filter(|t| !t.is_empty()),
        }
    }

    fn meta(&mut self, tag: &Tag) {
        let attr = |name: &str| {
            tag.attrs
                .iter()
                .find(|a| a.name.local.as_ref() == name)
                .map(|a| a.value.to_string())
        };
        let Some(content) = attr("content") else {
            return;
        };
        // Open Graph 按规范用 `property`，但不少站点写成 `name`；反过来 Twitter Card 也有写成 `property` 的。
        let key = attr("property")
            .or_else(|| attr("name"))
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let slot = match key.as_str() {
            "og:title" => &mut self.og_title,
            "twitter:title" => &mut self.twitter_title,
            "og:description" => &mut self.og_description,
            "twitter:description" => &mut self.twitter_description,
            "description" => &mut self.description,
            "og:site_name" => &mut self.site_name,
            "og:image" => &mut self.image,
            _ => return,
        };
        // 同一个属性写了几次的，取第一个。
        slot.get_or_insert(content);
    }

    fn tag(&mut self, tag: &Tag) -> TokenSinkResult<()> {
        let name = tag.name.as_ref();
        match (tag.kind, name) {
            (StartTag, "body") | (EndTag, "head") => self.done = true,
            (StartTag, "meta") => self.meta(tag),
            (StartTag, "title") => {
                self.in_title = Some(String::new());
                return TokenSinkResult::RawData(RawKind::Rcdata);
            }
            (EndTag, "title") => {
                if let Some(text) = self.in_title.take() {
                    self.title.get_or_insert(text);
                }
            }
            // 和浏览器一样切换分词状态：里面像标签的文字不当标签。
            (StartTag, "script") => return TokenSinkResult::RawData(RawKind::ScriptData),
            (StartTag, "style" | "noscript" | "xmp" | "iframe" | "noembed" | "noframes") => {
                return TokenSinkResult::RawData(RawKind::Rawtext);
            }
            (StartTag, "textarea") => return TokenSinkResult::RawData(RawKind::Rcdata),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

/// 合并空白，截到 `max` 个字符；空的算没有。
fn clean(text: &str, max: usize) -> Option<String> {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = joined.chars().take(max).collect();
    (!cut.is_empty()).then_some(cut)
}

#[derive(Default)]
struct Sink {
    found: RefCell<Found>,
}

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut found = self.found.borrow_mut();
        if found.done {
            return TokenSinkResult::Continue;
        }
        match token {
            TagToken(tag) => found.tag(&tag),
            CharacterTokens(text) => {
                if let Some(title) = found.in_title.as_mut() {
                    title.push_str(&text);
                }
                TokenSinkResult::Continue
            }
            _ => TokenSinkResult::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_graph_wins_over_twitter_and_title_and_body_is_ignored() {
        let page = br#"<!doctype html><html><head>
            <title>Plain &amp; title</title>
            <meta name="twitter:title" content="Twitter title">
            <meta property="og:title" content="  OG
              title ">
            <meta name="description" content="Plain description">
            <meta property="og:site_name" content="Site">
            <script>var s = '<meta property="og:image" content="/fake.png">';</script>
            <meta property="og:image" content="/real.png">
            </head><body><meta property="og:description" content="from body"></body></html>"#;
        let head = parse(page, None);
        assert_eq!(head.title.as_deref(), Some("OG title"));
        assert_eq!(head.description.as_deref(), Some("Plain description"));
        assert_eq!(head.site_name.as_deref(), Some("Site"));
        assert_eq!(head.image.as_deref(), Some("/real.png"));
    }

    #[test]
    fn the_title_tag_is_used_when_nothing_else_is_there() {
        let head = parse(b"<html><head><title>A <b> &lt;tag&gt;</title></head>", None);
        assert_eq!(head.title.as_deref(), Some("A <b> <tag>"));
        assert!(parse(b"<html><head></head><body>hi</body>", None).is_empty());
    }

    #[test]
    fn legacy_encodings_are_decoded_from_the_header_or_the_meta_tag() {
        // “你好”的 GBK 编码。
        let mut page = b"<html><head><meta charset=\"gbk\"><title>".to_vec();
        page.extend_from_slice(&[0xc4, 0xe3, 0xba, 0xc3]);
        page.extend_from_slice(b"</title></head>");
        assert_eq!(parse(&page, None).title.as_deref(), Some("你好"));
        assert_eq!(parse(&page, Some("GB2312")).title.as_deref(), Some("你好"));
    }
}
