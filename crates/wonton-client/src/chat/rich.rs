//! 把消息文字排成壳直接画的块和片段：别人发的（多半是 Bot）按 Markdown 解析，自己发的原样显示。
//!
//! 为了消息里的内容不能让 App 做额外的事：HTML 原样显示成文字，图片不去下载，只画成链接。链接的认法、补全和类别见
//! [`links`]；核心自己的 `wonton://` 通知链接（ADR-0041）交给系统。

use linkify::{LinkFinder, LinkKind};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use wonton_protocol::{RichBlock, RichBlockKind, RichLink, RichSpan};

use super::links::{self, LinkTargets};

/// 自己发的：只有一段，链接可以点。
pub(crate) fn plain(text: &str, links: &LinkTargets) -> Vec<RichBlock> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut builder = Builder::default();
    builder.text(text, links);
    builder.flush();
    builder.blocks
}

pub(crate) fn markdown(text: &str, links: &LinkTargets) -> Vec<RichBlock> {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut builder = Builder::default();
    let relaxed = relax_delimiters(text);
    for event in Parser::new_ext(&relaxed, options) {
        builder.event(event, links);
    }
    // 只有链接引用定义这类内容时解析不出东西，消息不能因此消失。
    if builder.blocks.is_empty() {
        return plain(text, links);
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
pub(super) const CJK_PUNCTUATION: [char; 15] = [
    '。', '，', '、', '；', '：', '！', '？', '）', '】', '》', '」', '』', '”', '’', '…',
];

#[derive(Default)]
struct Builder {
    blocks: Vec<RichBlock>,
    spans: Vec<RichSpan>,
    bold: u32,
    italic: u32,
    strike: u32,
    links: Vec<Option<RichLink>>,
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
    fn event(&mut self, event: Event<'_>, links: &LinkTargets) {
        match event {
            Event::Start(tag) => self.start(tag, links),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&without_zero_width(&text), links),
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

    fn start(&mut self, tag: Tag<'_>, links: &LinkTargets) {
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
            Tag::Link { dest_url, .. } => self
                .links
                .push(links.explicit(&without_zero_width(&dest_url))),
            Tag::Image { dest_url, .. } => {
                self.links
                    .push(links.explicit(&without_zero_width(&dest_url)));
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
            self.span(&link.url.clone(), Some(link));
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

    fn text(&mut self, text: &str, links: &LinkTargets) {
        if let Some(code) = &mut self.code {
            code.push_str(text);
        } else if self.table.is_some() || self.active_link().is_some() {
            self.span(text, None);
        } else {
            // 号码先切出来：linkify 会把 `+1.555.010.7000` 的点分号码当成 IPv4 地址，号码得压过它。
            let mut at = 0;
            for phone in links::phones(text) {
                self.links_in(&text[at..phone.start], links);
                self.dial(&text[phone.clone()], links);
                at = phone.end;
            }
            self.links_in(&text[at..], links);
        }
    }

    /// 一段没有号码的文字里的网址和邮箱。
    fn links_in(&mut self, text: &str, links: &LinkTargets) {
        let mut finder = LinkFinder::new();
        finder.kinds(&[LinkKind::Url, LinkKind::Email]);
        finder.url_must_have_scheme(false);
        for part in finder.spans(text) {
            match part.kind() {
                None => self.span(part.as_str(), None),
                Some(_) => self.link(part.as_str(), links),
            }
        }
    }

    /// 一段号码：`tel:` 明写的和裸写的都交给 `tel:` 的解析。
    fn dial(&mut self, number: &str, links: &LinkTargets) {
        match links.link(number, &LinkKind::Url) {
            Some(link) => self.span(number, Some(link)),
            None => self.span(number, None),
        }
    }

    /// 一个链接片段。`linkify` 会把中文标点隔开的几个地址连成一段，先按标点切开，每一段各自判断。
    fn link(&mut self, found: &str, links: &LinkTargets) {
        let mut rest = found;
        while !rest.is_empty() {
            let cut = rest.find(CJK_PUNCTUATION).unwrap_or(rest.len());
            let (part, tail) = rest.split_at(cut);
            self.piece(part, links);
            let punctuation: String = tail.chars().take(1).collect();
            self.span(&punctuation, None);
            rest = &tail[punctuation.len()..];
        }
    }

    /// 切开的一段：可能是地址、邮箱，也可能只是紧跟着网址的汉字。
    fn piece(&mut self, part: &str, links: &LinkTargets) {
        if part.is_empty() {
            return;
        }
        // 按这一段自己的样子认，不沿用整段的类别。
        let kind = if part.contains('@') && !part.contains("://") {
            LinkKind::Email
        } else {
            LinkKind::Url
        };
        match links.link(part, &kind) {
            Some(link) => self.span(part, Some(link)),
            None => self.span(part, None),
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

    fn span(&mut self, text: &str, link: Option<RichLink>) {
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

    fn active_link(&self) -> Option<&RichLink> {
        self.links.iter().rev().find_map(Option::as_ref)
    }

    fn style(&self, link: Option<RichLink>) -> RichSpan {
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
    use wonton_protocol::{MatrixTarget, RichLinkKind};

    fn md(text: &str) -> Vec<RichBlock> {
        markdown(text, &LinkTargets::default())
    }

    fn pl(text: &str) -> Vec<RichBlock> {
        plain(text, &LinkTargets::default())
    }

    fn links(blocks: &[RichBlock]) -> Vec<&str> {
        blocks
            .iter()
            .flat_map(|block| &block.spans)
            .filter_map(|span| span.link.as_ref())
            .map(|link| link.url.as_str())
            .collect()
    }

    #[test]
    fn copied_text_is_what_the_reader_sees() {
        let blocks = md("**重点** 和 `code`\n\n- 第一项\n- [链接](https://example.com)");
        assert_eq!(copy_text(&blocks), "重点 和 code\n• 第一项\n• 链接");
    }

    #[test]
    fn bot_text_loses_its_markup() {
        let blocks = md("The launch passphrase in `rehearsal-notes.txt` is **`lantern-42`**.");
        assert_eq!(
            flatten(&blocks),
            "The launch passphrase in rehearsal-notes.txt is lantern-42."
        );
        let spans = &blocks[0].spans;
        assert!(spans[1].code && !spans[1].bold);
        assert!(spans[3].code && spans[3].bold);
    }

    fn bold_parts(text: &str) -> (String, Vec<String>) {
        let blocks = md(text);
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
        assert_eq!(flatten(&md("2*(3+4)*5")), "2*(3+4)*5");
        assert_eq!(flatten(&md("`a**“b”**c`")), "a**“b”**c");
    }

    #[test]
    fn own_text_stays_as_typed() {
        let blocks = pl("**不是粗体** `也不是代码`");
        assert_eq!(flatten(&blocks), "**不是粗体** `也不是代码`");
        assert_eq!(blocks[0].spans.len(), 1);
    }

    #[test]
    fn only_web_mail_and_app_links_can_be_opened() {
        let blocks = md(
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

    fn kinds(blocks: &[RichBlock]) -> Vec<(String, String, RichLinkKind)> {
        blocks
            .iter()
            .flat_map(|block| &block.spans)
            .filter_map(|span| {
                let link = span.link.as_ref()?;
                Some((span.text.clone(), link.url.clone(), link.kind.clone()))
            })
            .collect()
    }

    /// 行为：正文里不带协议的网址、邮箱、插件页、matrix.to、号码和 App 链接都成了链接，类别由核心给。
    #[test]
    fn links_are_found_and_classified() {
        let targets =
            links::test_targets(&[("app.grafana", "http://app-grafana.home.example:18480/")]);
        let blocks = markdown(
            "看 example.com/a，写给 test@example.com；插件 http://app-grafana.home.example:18480/explore，\
             用户 https://matrix.to/#/@family:home.example，事件 https://matrix.to/#/!abc:home.example/$e，\
             电话 +86 138 0013 8000，App wonton://settings/peers。",
            &targets,
        );
        assert_eq!(
            kinds(&blocks),
            [
                (
                    "example.com/a".to_owned(),
                    "http://example.com/a".to_owned(),
                    RichLinkKind::External
                ),
                (
                    "test@example.com".to_owned(),
                    "mailto:test@example.com".to_owned(),
                    RichLinkKind::Email
                ),
                (
                    "http://app-grafana.home.example:18480/explore".to_owned(),
                    "http://app-grafana.home.example:18480/explore".to_owned(),
                    RichLinkKind::Plugin {
                        plugin_id: "app.grafana".to_owned()
                    }
                ),
                (
                    "https://matrix.to/#/@family:home.example".to_owned(),
                    "https://matrix.to/#/@family:home.example".to_owned(),
                    RichLinkKind::Matrix {
                        target: MatrixTarget::User {
                            user_id: "@family:home.example".to_owned()
                        }
                    }
                ),
                (
                    "https://matrix.to/#/!abc:home.example/$e".to_owned(),
                    "https://matrix.to/#/!abc:home.example/$e".to_owned(),
                    RichLinkKind::Matrix {
                        target: MatrixTarget::Room {
                            room_id: "!abc:home.example".to_owned(),
                            event_id: Some("$e".to_owned())
                        }
                    }
                ),
                (
                    "+86 138 0013 8000".to_owned(),
                    "tel:+8613800138000".to_owned(),
                    RichLinkKind::Phone
                ),
                (
                    "wonton://settings/peers".to_owned(),
                    "wonton://settings/peers".to_owned(),
                    RichLinkKind::Other
                ),
            ]
        );
        // 切掉的中文标点留在文字里，不掉字。
        assert_eq!(
            flatten(&blocks),
            "看 example.com/a，写给 test@example.com；插件 http://app-grafana.home.example:18480/explore，\
             用户 https://matrix.to/#/@family:home.example，事件 https://matrix.to/#/!abc:home.example/$e，\
             电话 +86 138 0013 8000，App wonton://settings/peers。"
        );
    }

    /// 只认真的顶级域：文件名（`main.rs`）和不在公共后缀表里的写法不是链接。
    #[test]
    fn file_names_are_not_links() {
        let none = md("改了 main.rs、README.md 和 config.yaml，用 Vue.js");
        assert_eq!(kinds(&none), []);
        // 有路径、端口或者 `www.` 就是网址。
        let some = md("看 example.com/a 和 x.io、example.cn，还有 example.md/x 和 example.md:8080");
        assert_eq!(
            kinds(&some)
                .iter()
                .map(|(_, _, kind)| kind.clone())
                .collect::<Vec<_>>(),
            [
                RichLinkKind::External,
                RichLinkKind::External,
                RichLinkKind::External,
                RichLinkKind::External,
                RichLinkKind::External,
            ]
        );
    }

    /// 认不出的不猜：单字母顶级域、版本号、算式里的加号都不是链接。
    #[test]
    fn short_hosts_and_formulas_stay_plain() {
        let blocks = md("a.b 里 v1.2.3，C++17 和 1+2，tel:+1234567");
        assert_eq!(
            kinds(&blocks),
            [(
                "tel:+1234567".to_owned(),
                "tel:+1234567".to_owned(),
                RichLinkKind::Phone
            )]
        );
        // 点分组的号码和 `tel:` 明写的点分号码都是电话，不是 IPv4 网页。
        let dotted = md("打 +1.555.010.7000 或 tel:+1.555.010.7000");
        assert_eq!(
            kinds(&dotted),
            [
                (
                    "+1.555.010.7000".to_owned(),
                    "tel:+15550107000".to_owned(),
                    RichLinkKind::Phone
                ),
                (
                    "tel:+1.555.010.7000".to_owned(),
                    "tel:+15550107000".to_owned(),
                    RichLinkKind::Phone
                ),
            ]
        );
        assert_eq!(
            flatten(&blocks),
            "a.b 里 v1.2.3，C++17 和 1+2，tel:+1234567"
        );
    }

    /// 同一个插件站点上的地址只有装了那个插件才算插件页，别的都是外部网页。
    #[test]
    fn plugin_pages_need_the_plugin() {
        let none = md("http://app-grafana.home.example:18480/explore");
        assert_eq!(kinds(&none)[0].2, RichLinkKind::External);
        let blocked = markdown(
            "[插件](http://app-grafana.home.example:18480/)",
            &links::test_targets(&[("app.grafana", "http://app-grafana.home.example:18480/")]),
        );
        assert_eq!(
            kinds(&blocked)[0].2,
            RichLinkKind::Plugin {
                plugin_id: "app.grafana".to_owned()
            }
        );
    }

    /// Markdown 目标要是相对路径就不是网址，文字还在；带协议的照常是链接。
    #[test]
    fn relative_markdown_targets_are_not_links() {
        let blocks =
            md("见 [说明](main.rs) 和 [文档](docs/a.md)，站 [example](https://example.com)");
        assert_eq!(links(&blocks), ["https://example.com/"]);
        assert_eq!(flatten(&blocks), "见 说明 和 文档，站 example");
    }

    /// 裸写的 App 链接也是链接：核心的提醒里直接贴 `wonton://…`。
    #[test]
    fn a_bare_app_link_is_a_link() {
        let blocks = md("其他服务器 wonton://settings/peers 看看");
        assert_eq!(links(&blocks), ["wonton://settings/peers"]);
    }

    #[test]
    fn html_is_shown_not_interpreted() {
        let blocks = md("前 <img src=x onerror=y> 后\n\n<script>alert(1)</script>");
        assert_eq!(
            flatten(&blocks),
            "前 <img src=x onerror=y> 后\n<script>alert(1)</script>"
        );
    }

    #[test]
    fn tables_have_a_preview_too() {
        assert_eq!(flatten(&md("| a | b |\n| - | - |\n| c | d |")), "a b c d");
    }

    #[test]
    fn nothing_parsed_still_shows_the_message() {
        assert_eq!(
            flatten(&md("[a]: https://example.com")),
            "[a]: https://example.com"
        );
    }
}
