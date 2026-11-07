//! 文件编号和聊天里的文件链接（ADR-0020 的“链接”）。

use std::fmt;
use std::ops::Range;

use serde::{Deserialize, Serialize};

const LINK_PREFIX: &str = "wonton://file/";
const ID_LEN: usize = 26;
/// RFC 4648 的 base32，小写。
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// 文件的编号：128 位随机数的小写 base32，26 个字符。它不是凭据，下载还要设备身份和访问范围。
///
/// 跨 FFI 时是普通字符串，壳原样交回核心，不解析。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FileId(String);

/// 不是合法的文件编号。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a valid file id")]
pub struct InvalidFileId;

impl FileId {
    /// 由 16 个随机字节生成编号。不可猜全靠这些字节：调用方要用密码学安全的随机数。
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        let value = u128::from_be_bytes(bytes);
        // 128 位编成 26 个字符：前 25 个各 5 位，最后一个是剩下的 3 位加 2 个填充的 0。
        let id = (0..ID_LEN)
            .map(|index| {
                let digit = match index {
                    25 => (value & 0b111) << 2,
                    _ => (value >> (123 - 5 * index)) & 0b1_1111,
                };
                char::from(ALPHABET[digit as usize])
            })
            .collect();
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 这个文件的链接，只有 `wonton://file/<编号>` 这一段，不带地址、文件名和大小。
    pub fn link(&self) -> String {
        format!("{LINK_PREFIX}{}", self.0)
    }

    /// 解析一个完整的链接：前后不能有别的字符。从消息正文里找链接用 [`extract_file_links`]。
    pub fn from_link(link: &str) -> Option<Self> {
        link.strip_prefix(LINK_PREFIX)?.parse().ok()
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for FileId {
    type Err = InvalidFileId;

    /// 最后一个字符不查填充位：模型抄错一个字符时，应该走到 `file_unknown`、卡片如实显示，而不是留在正文里。
    fn from_str(id: &str) -> Result<Self, InvalidFileId> {
        let valid = id.len() == ID_LEN && id.bytes().all(|byte| ALPHABET.contains(&byte));
        valid.then(|| Self(id.to_owned())).ok_or(InvalidFileId)
    }
}

impl TryFrom<String> for FileId {
    type Error = InvalidFileId;

    fn try_from(id: String) -> Result<Self, InvalidFileId> {
        id.parse()
    }
}

impl From<FileId> for String {
    fn from(id: FileId) -> Self {
        id.0
    }
}

// 壳交回来的编号也要校验：它会拼进 `/_wonton/files/{id}` 这样的路径。
#[cfg(feature = "uniffi")]
uniffi::custom_type!(FileId, String);

/// [`extract_file_links`] 的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileLinks {
    /// 去掉链接后要显示的文字；消息只有链接时为空。没有链接时和原文完全一样。
    pub body: String,
    /// 认出的文件，按第一次出现的顺序，不重复。
    pub files: Vec<FileId>,
}

/// 从消息的纯文本 `body` 里认出文件链接，连同包着它的 Markdown 语法一起去掉：`[名字](链接)`、`![名字](链接)`、
/// `<链接>`、`` `链接` ``、`**链接**`。
///
/// 规则是严格的：scheme 和路径大小写一致、编号正好 26 个合法字符，前后紧贴着网址里才有的字符时不算（别的 URL 里
/// 的一段，或者带了扩展名、查询串的变体）。中文和标点可以紧贴着链接，模型的回复常常不加空格。
pub fn extract_file_links(text: &str) -> FileLinks {
    let links = find_links(text);
    if links.is_empty() {
        return FileLinks {
            body: text.to_owned(),
            files: Vec::new(),
        };
    }
    let mut files = Vec::new();
    let mut spans = Vec::new();
    for (range, id) in links {
        spans.push(with_wrapper(text, range));
        if !files.contains(&id) {
            files.push(id);
        }
    }
    FileLinks {
        body: remove_spans(text, spans),
        files,
    }
}

fn find_links(text: &str) -> Vec<(Range<usize>, FileId)> {
    text.match_indices(LINK_PREFIX)
        .filter_map(|(start, _)| {
            let id_start = start + LINK_PREFIX.len();
            let end = id_start + ID_LEN;
            let id = text.get(id_start..end)?.parse().ok()?;
            let free = !continues_before(&text[..start]) && !continues_after(&text[end..]);
            free.then_some((start..end, id))
        })
        .collect()
}

fn continues_before(before: &str) -> bool {
    before
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_alphanumeric() || "+-./=%&_~@#?".contains(c))
}

fn continues_after(after: &str) -> bool {
    let mut chars = after.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() || "-_/%=&+~@#".contains(c) => true,
        // 句末的标点不算，后面还跟着网址字符才算：`….jpg`、`…?name=a`。
        Some('.' | '?' | ':') => chars.next().is_some_and(|c| c.is_ascii_alphanumeric()),
        _ => false,
    }
}

/// 把链接的范围扩大到包着它的 Markdown 语法。
fn with_wrapper(text: &str, link: Range<usize>) -> Range<usize> {
    let (before, after) = (&text[..link.start], &text[link.end..]);
    if let Some(span) = markdown_link(before, after) {
        return span.start..link.end + span.end;
    }
    for (open, close) in [("<", ">"), ("`", "`"), ("**", "**")] {
        if before.ends_with(open) && after.starts_with(close) {
            return link.start - open.len()..link.end + close.len();
        }
    }
    link
}

/// `[名字](链接)` 里链接之外的部分：返回开头在 `before` 里的下标，和结尾在 `after` 里的下标。
fn markdown_link(before: &str, after: &str) -> Option<Range<usize>> {
    let end = markdown_link_tail(after)?;
    let label_end = before.strip_suffix("](")?.len();
    let label_start = before[..label_end].rfind('[')?;
    if before[label_start..label_end].contains([']', '\n']) {
        return None;
    }
    let image = before[..label_start].ends_with('!');
    Some(label_start - usize::from(image)..end)
}

/// 链接后面的 `)` 或 ` "标题")` 有多长。
fn markdown_link_tail(after: &str) -> Option<usize> {
    if after.starts_with(')') {
        return Some(1);
    }
    let title = after.strip_prefix(" \"")?;
    let quote = title.find('"')?;
    let closed = title[quote + 1..].starts_with(')') && !title[..quote].contains('\n');
    closed.then_some(" \"".len() + quote + "\")".len())
}

/// 去掉这些范围（一个范围不跨行）。链接占一整行时这一行也不留，文字就不会隔着一片空行。
fn remove_spans(text: &str, mut spans: Vec<Range<usize>>) -> String {
    // 名字里也写了链接时，外面的范围盖住里面的。
    spans.sort_by_key(|span| span.start);
    let mut spans = spans.into_iter().peekable();
    let mut body = String::new();
    let mut skip_blank = false;
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let mut kept = String::new();
        let mut cursor = line_start;
        while let Some(span) = spans.next_if(|span| span.start < line_end) {
            if cursor == line_start {
                kept.push_str(&text[cursor..span.start]);
            } else if span.start > cursor {
                push_after_link(&mut kept, &text[cursor..span.start]);
            }
            cursor = cursor.max(span.end);
        }
        let touched = cursor > line_start;
        line_start = line_end;
        if !touched {
            if !(skip_blank && line.trim().is_empty()) {
                skip_blank = false;
                body.push_str(line);
            }
            continue;
        }
        push_after_link(&mut kept, &text[cursor..line_end]);
        if is_empty_line(&kept) {
            skip_blank = body.is_empty() || body.ends_with("\n\n");
        } else {
            skip_blank = false;
            body.push_str(kept.trim_end());
            body.push('\n');
        }
    }
    body.trim().to_owned()
}

/// 接上链接后面的文字，接缝处不留两个空格，行首不多出空格。
fn push_after_link(kept: &mut String, piece: &str) {
    if kept.is_empty() || kept.ends_with([' ', '\t']) {
        kept.push_str(piece.trim_start_matches([' ', '\t']));
    } else {
        kept.push_str(piece);
    }
}

/// 去掉链接后这一行什么都不剩，或者只剩列表的标记（`- 链接`、`1. 链接`）。
fn is_empty_line(line: &str) -> bool {
    let line = line.trim();
    let ordered = line
        .strip_suffix(['.', ')'])
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()));
    line.is_empty() || ordered || ["-", "*", "+", "•"].contains(&line)
}
