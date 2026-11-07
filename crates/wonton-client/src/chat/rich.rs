//! 把消息文字排成壳直接画的块和片段：别人发的（多半是 Bot）按 Markdown 解析，自己发的原样显示。
//!
//! 为了消息里的内容不能让 App 做额外的事：HTML 原样显示成文字，图片不去下载，只画成链接，链接只认 http、https、mailto 和云吞自己的 `wonton://`（核心的通知带 App 链接，ADR-0041）。

use linkify::{LinkFinder, LinkKind};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use url::Url;
use wonton_protocol::{RichBlock, RichBlockKind, RichSpan};

/// 自己发的：只有一段，网址可以点。
pub(crate) fn plain(text: &str) -> Vec<RichBlock> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut builder = Builder::default();
    builder.text(text);
    builder.flush();
    builder.blocks
}

pub(crate) fn markdown(text: &str) -> Vec<RichBlock> {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut builder = Builder::default();
    let relaxed = relax_delimiters(text);
    for event in Parser::new_ext(&relaxed, options) {
        builder.event(event);
    }
    // 只有链接引用定义这类内容时解析不出东西，消息不能因此消失。
    if builder.blocks.is_empty() {
        return plain(text);
    }
    builder.blocks
}

/// 列表预览和通知用的文字，去掉了排版，换行还在：这两处各自决定显示几行。
pub(crate) fn flatten(blocks: &[RichBlock]) -> String {
    let texts = blocks.iter().map(|block| match &block.kind {
        RichBlockKind::Table { rows } => {
            rows.iter().flatten().cloned().collect::<Vec<_>>().join(" ")
        }
        _ => block.spans.iter().map(|span| span.text.as_str()).collect(),
    });
    texts
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

const ZERO_WIDTH_SPACE: char = '\u{200B}';

/// CommonMark 判断定界符时假定词之间有空格：`这是**“重点”**吧`、`**注意：**内容` 里的 `**` 一边贴标点、一边贴中文，
/// 不算定界符，不但原样显示，还会和后面的 `**` 错配。pulldown-cmark 没有这方面的选项，所以在定界符靠标点的一侧插一个
/// 零宽空格，它既不算空白也不算标点，解析器就认了；输出前再全部去掉（[`Builder::event`]）。纯 ASCII 的不动，
/// `2*(3+4)*5` 不能变成斜体。
fn relax_delimiters(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut relaxed = String::with_capacity(text.len());
    let mut start = 0;
    while start < chars.len() {
        let current = chars[start];
        if current != '*' && current != '~' {
            relaxed.push(current);
            start += 1;
            continue;
        }
        let length = chars[start..].iter().take_while(|&&c| c == current).count();
        let end = start + length;
        let before = start.checked_sub(1).map(|index| chars[index]);
        let after = chars.get(end).copied();
        if needs_relaxing(before, after) {
            relaxed.push(ZERO_WIDTH_SPACE);
        }
        relaxed.extend(&chars[start..end]);
        if needs_relaxing(after, before) {
            relaxed.push(ZERO_WIDTH_SPACE);
        }
        start = end;
    }
    relaxed
}

/// 定界符的 `inner` 一侧是标点、`outer` 一侧是文字，而且至少有一边不是 ASCII。
fn needs_relaxing(inner: Option<char>, outer: Option<char>) -> bool {
    let (Some(inner), Some(outer)) = (inner, outer) else {
        return false;
    };
    is_punctuation(inner)
        && !is_punctuation(outer)
        && !outer.is_whitespace()
        && !(inner.is_ascii() && outer.is_ascii())
}

/// 标准库没有 Unicode 标点的判断，这里只列中文里会用到的。漏掉的字符只是退回 CommonMark 原本的行为。
fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(
            c,
            '\u{2010}'..='\u{2027}'
                | '\u{2030}'..='\u{205E}'
                | '\u{3001}'..='\u{303F}'
                | '\u{FF01}'..='\u{FF0F}'
                | '\u{FF1A}'..='\u{FF20}'
                | '\u{FF3B}'..='\u{FF40}'
                | '\u{FF5B}'..='\u{FF65}'
        )
}

/// linkify 只认 ASCII 标点是网址的结尾，中文句子里紧跟网址的标点会被算进地址。
const CJK_PUNCTUATION: [char; 15] = [
    '。', '，', '、', '；', '：', '！', '？', '）', '】', '》', '」', '』', '”', '’', '…',
];

#[derive(Default)]
struct Builder {
    blocks: Vec<RichBlock>,
    spans: Vec<RichSpan>,
    bold: u32,
    italic: u32,
    strike: u32,
    links: Vec<Option<String>>,
    /// 每层列表下一项的序号，无序列表为 `None`。
    lists: Vec<Option<u64>>,
    /// 当前列表项的符号，还没画出去。
    marker: Option<String>,
    quotes: u32,
    heading: bool,
    code: Option<String>,
    table: Option<Table>,
    /// 图片开始时的片段数，用来发现图片没有替代文字。
    image_start: usize,
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: String,
}

impl Builder {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&without_zero_width(&text)),
            Event::Code(text) => self.inline_code(&without_zero_width(&text)),
            // HTML 不解释，原样显示。
            Event::Html(text) | Event::InlineHtml(text) => {
                self.span(without_zero_width(&text).trim_end(), None);
            }
            Event::SoftBreak | Event::HardBreak => self.span("\n", None),
            Event::Rule => {
                self.flush();
                self.push(RichBlockKind::Rule, Vec::new());
            }
            Event::TaskListMarker(done) => {
                self.marker = Some(if done { "☑" } else { "☐" }.to_owned());
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { .. } => self.heading = true,
            Tag::BlockQuote(_) => {
                self.flush();
                self.quotes += 1;
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.code = Some(String::new());
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(start);
            }
            Tag::Item => self.marker = Some(self.next_marker()),
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Table::default());
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.links.push(allowed_link(&dest_url)),
            Tag::Image { dest_url, .. } => {
                self.links.push(allowed_link(&dest_url));
                self.image_start = self.spans.len();
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Item | TagEnd::HtmlBlock => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.heading = false;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quotes -= 1;
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                self.push_code(code.trim_end_matches('\n'));
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::TableCell => {
                if let Some(table) = &mut self.table {
                    let cell = std::mem::take(&mut table.cell);
                    table.row.push(cell);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(table) = &mut self.table {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.push(RichBlockKind::Table { rows: table.rows }, Vec::new());
                }
            }
            TagEnd::Emphasis => self.italic -= 1,
            TagEnd::Strong => self.bold -= 1,
            TagEnd::Strikethrough => self.strike -= 1,
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::Image => self.end_image(),
            _ => {}
        }
    }

    /// 图片画成链接，文字是替代文字；没有替代文字时显示地址，不让消息里的内容凭空消失。
    fn end_image(&mut self) {
        let link = self.links.pop().flatten();
        if self.image_start == self.spans.len()
            && let Some(link) = link
        {
            self.span(&link.clone(), Some(link));
        }
    }

    fn next_marker(&mut self) -> String {
        match self.lists.last_mut() {
            Some(Some(next)) => {
                let marker = format!("{next}.");
                *next += 1;
                marker
            }
            _ => "•".to_owned(),
        }
    }

    fn text(&mut self, text: &str) {
        if let Some(code) = &mut self.code {
            code.push_str(text);
        } else if self.table.is_some() || self.active_link().is_some() {
            self.span(text, None);
        } else {
            let mut finder = LinkFinder::new();
            finder.kinds(&[LinkKind::Url]);
            for part in finder.spans(text) {
                let found = part.as_str();
                if part.kind().is_none() {
                    self.span(found, None);
                    continue;
                }
                let (url, rest) =
                    found.split_at(found.find(CJK_PUNCTUATION).unwrap_or(found.len()));
                self.span(url, allowed_link(url));
                self.span(rest, None);
            }
        }
    }

    fn inline_code(&mut self, text: &str) {
        if let Some(table) = &mut self.table {
            table.cell.push_str(text);
            return;
        }
        let span = RichSpan {
            text: text.to_owned(),
            code: true,
            ..self.style(None)
        };
        self.add(span);
    }

    fn span(&mut self, text: &str, link: Option<String>) {
        if text.is_empty() {
            return;
        }
        if let Some(table) = &mut self.table {
            table.cell.push_str(&text.replace('\n', " "));
            return;
        }
        let span = RichSpan {
            text: text.to_owned(),
            ..self.style(link)
        };
        self.add(span);
    }

    fn active_link(&self) -> Option<&String> {
        self.links.iter().rev().find_map(Option::as_ref)
    }

    fn style(&self, link: Option<String>) -> RichSpan {
        RichSpan {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            link: link.or_else(|| self.active_link().cloned()),
            ..RichSpan::default()
        }
    }

    /// 样式相同的接上去：解析器会把一段文字切成好几个事件。
    fn add(&mut self, span: RichSpan) {
        if let Some(last) = self.spans.last_mut() {
            let same = RichSpan {
                text: last.text.clone(),
                ..span.clone()
            };
            if same == *last {
                last.text.push_str(&span.text);
                return;
            }
        }
        self.spans.push(span);
    }

    /// 结束当前的文字。列表项没有文字时符号也要画，项里可能只有代码块。
    fn flush(&mut self) {
        if self.spans.is_empty() && self.marker.is_none() {
            return;
        }
        let kind = match self.marker.take() {
            Some(marker) => RichBlockKind::ListItem { marker },
            None if self.heading => RichBlockKind::Heading,
            None => RichBlockKind::Paragraph,
        };
        let spans = std::mem::take(&mut self.spans);
        self.push(kind, spans);
    }

    fn push_code(&mut self, text: &str) {
        let spans = vec![RichSpan {
            text: text.to_owned(),
            ..RichSpan::default()
        }];
        self.push(RichBlockKind::Code, spans);
    }

    fn push(&mut self, kind: RichBlockKind, spans: Vec<RichSpan>) {
        // 列表项自己在外层列表的缩进里，项里的其他内容对齐到项的文字下面。
        let depth = match kind {
            RichBlockKind::ListItem { .. } => self.lists.len().saturating_sub(1),
            _ => self.lists.len(),
        };
        self.blocks.push(RichBlock {
            kind,
            depth: u32::try_from(depth).unwrap_or(u32::MAX),
            quoted: self.quotes > 0,
            spans,
        });
    }
}

fn without_zero_width(text: &str) -> String {
    text.replace(ZERO_WIDTH_SPACE, "")
}

fn allowed_link(url: &str) -> Option<String> {
    let parsed = Url::parse(&without_zero_width(url)).ok()?;
    matches!(parsed.scheme(), "http" | "https" | "mailto" | "wonton").then(|| parsed.to_string())
}

/// 复制出去的文字：看到什么复制什么，不带 Markdown 的符号。
pub(crate) fn copy_text(blocks: &[RichBlock]) -> String {
    let lines: Vec<String> = blocks
        .iter()
        .filter_map(|block| {
            let text: String = block.spans.iter().map(|span| span.text.as_str()).collect();
            match &block.kind {
                RichBlockKind::ListItem { marker } => Some(format!("{marker} {text}")),
                RichBlockKind::Table { rows } => Some(
                    rows.iter()
                        .map(|row| row.join("\t"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                RichBlockKind::Rule => None,
                _ => Some(text),
            }
        })
        .collect();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(blocks: &[RichBlock]) -> Vec<&str> {
        blocks
            .iter()
            .flat_map(|block| &block.spans)
            .filter_map(|span| span.link.as_deref())
            .collect()
    }

    #[test]
    fn copied_text_is_what_the_reader_sees() {
        let blocks = markdown("**重点** 和 `code`\n\n- 第一项\n- [链接](https://example.com)");
        assert_eq!(copy_text(&blocks), "重点 和 code\n• 第一项\n• 链接");
    }

    #[test]
    fn bot_text_loses_its_markup() {
        let blocks =
            markdown("The launch passphrase in `rehearsal-notes.txt` is **`lantern-42`**.");
        assert_eq!(
            flatten(&blocks),
            "The launch passphrase in rehearsal-notes.txt is lantern-42."
        );
        let spans = &blocks[0].spans;
        assert!(spans[1].code && !spans[1].bold);
        assert!(spans[3].code && spans[3].bold);
    }

    fn bold_parts(text: &str) -> (String, Vec<String>) {
        let blocks = markdown(text);
        let bold = blocks[0]
            .spans
            .iter()
            .filter(|span| span.bold)
            .map(|span| span.text.clone())
            .collect();
        (flatten(&blocks), bold)
    }

    #[test]
    fn bold_works_next_to_chinese_punctuation() {
        assert_eq!(
            bold_parts("这是**“重点”**吧，这是**普通粗体**吧"),
            (
                "这是“重点”吧，这是普通粗体吧".to_owned(),
                vec!["“重点”".to_owned(), "普通粗体".to_owned()]
            )
        );
        assert_eq!(
            bold_parts("**注意：**后面的内容"),
            ("注意：后面的内容".to_owned(), vec!["注意：".to_owned()])
        );
    }

    #[test]
    fn delimiters_stay_literal_where_nothing_was_meant() {
        assert_eq!(flatten(&markdown("2*(3+4)*5")), "2*(3+4)*5");
        assert_eq!(flatten(&markdown("`a**“b”**c`")), "a**“b”**c");
    }

    #[test]
    fn own_text_stays_as_typed() {
        let blocks = plain("**不是粗体** `也不是代码`");
        assert_eq!(flatten(&blocks), "**不是粗体** `也不是代码`");
        assert_eq!(blocks[0].spans.len(), 1);
    }

    #[test]
    fn only_web_mail_and_app_links_can_be_opened() {
        let blocks = markdown(
            "[a](javascript:alert(1)) [b](file:///etc/passwd) [c](https://example.com) \
             [d](mailto:a@example.com) [e](wonton://settings/peers) https://example.org/x。 \
             ![f](http://example.com/i.png)",
        );
        assert_eq!(
            links(&blocks),
            [
                "https://example.com/",
                "mailto:a@example.com",
                "wonton://settings/peers",
                "https://example.org/x",
                "http://example.com/i.png",
            ]
        );
    }

    /// 裸写的 App 链接也是链接：核心的提醒里直接贴 `wonton://…`。
    #[test]
    fn a_bare_app_link_is_a_link() {
        let blocks = markdown("其他服务器 wonton://settings/peers 看看");
        assert_eq!(links(&blocks), ["wonton://settings/peers"]);
    }

    #[test]
    fn html_is_shown_not_interpreted() {
        let blocks = markdown("前 <img src=x onerror=y> 后\n\n<script>alert(1)</script>");
        assert_eq!(
            flatten(&blocks),
            "前 <img src=x onerror=y> 后\n<script>alert(1)</script>"
        );
    }

    #[test]
    fn tables_have_a_preview_too() {
        assert_eq!(
            flatten(&markdown("| a | b |\n| - | - |\n| c | d |")),
            "a b c d"
        );
    }

    #[test]
    fn nothing_parsed_still_shows_the_message() {
        assert_eq!(
            flatten(&markdown("[a]: https://example.com")),
            "[a]: https://example.com"
        );
    }
}
