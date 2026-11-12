//! 动态摘要、话题开局说明和 notice 的文字，按房间主人的语言取 `locales/*/topics.ftl`。

use wonton_l10n::{Arg, Lang};

use crate::l10n::text;
use crate::topics::SessionKey;
use crate::topics::board::{Board, NAME_MAX_CHARS, Row, Status};

/// 动态里最多列最近几条变化（ADR-0058：内容要截短）。
const DIGEST_MAX_CHANGES: usize = 5;
/// 摘要和 notice 里文件超过几个只写个数（ADR-0058）。
const FILE_LIST_MAX: usize = 5;
/// 话题开局带话题起点之前的主对话消息条数。
pub(crate) const OPENING_CONTEXT_MESSAGES: usize = 6;
/// 话题开局里每条片段的截短长度。
const CONTEXT_MAX_CHARS: usize = 120;
/// 其他会话从 viewer 上一轮开始到现在的工作板变化；没有变化返回 `None`。
pub(crate) fn digest(board: &Board, viewer: &SessionKey, lang: Lang) -> Option<String> {
    let base = board.row(viewer).map(|row| row.last_seen_seq).unwrap_or(0);
    let changes: Vec<String> = board
        .changes_for(viewer, base)
        .into_iter()
        .rev()
        .take(DIGEST_MAX_CHANGES)
        .rev()
        .map(|change| format!("- {}", change.text))
        .collect();
    let live: Vec<String> = board
        .rows()
        .iter()
        .filter(|row| &row.key != viewer)
        .filter_map(|row| live_line(row, lang).map(|line| format!("- {line}")))
        .collect();
    if changes.is_empty() && live.is_empty() {
        return None;
    }
    let mut lines = vec![text(lang, "digest-header", &[])];
    lines.extend(changes);
    lines.extend(live);
    Some(lines.join("\n"))
}

/// 还在做事的会话在动态里的一行：“现在在做什么”。
fn live_line(row: &Row, lang: Lang) -> Option<String> {
    let name = Arg::from(row.name.as_str());
    match row.status {
        Status::Working => Some(text(
            lang,
            "digest-working",
            &[("name", name), ("doing", Arg::from(row.doing.as_str()))],
        )),
        Status::WaitingApproval => Some(text(lang, "digest-waiting", &[("name", name)])),
        Status::Queued => Some(text(lang, "digest-queued", &[("name", name)])),
        Status::Idle | Status::Finished => None,
    }
}

/// 话题开局说明：主对话的片段（起点那条和它前面最多 6 条）、当前工作板、分工规矩、
/// 用户的第一句。`viewer` 是话题自己，工作板快照里不带它。
pub(crate) fn opening(
    board: &Board,
    viewer: &SessionKey,
    root: &str,
    preceding: &[String],
    lang: Lang,
) -> String {
    let name = clip_plain(root, NAME_MAX_CHARS);
    let mut lines = vec![text(lang, "digest-intro", &[("name", Arg::from(name))])];
    lines.push(text(lang, "digest-preceding-header", &[]));
    let start = preceding.len().saturating_sub(OPENING_CONTEXT_MESSAGES);
    for message in &preceding[start..] {
        lines.push(format!("- {}", clip(message, CONTEXT_MAX_CHARS)));
    }
    lines.push(format!("- {}", clip(root, CONTEXT_MAX_CHARS)));
    lines.push(String::new());
    lines.push(text(lang, "digest-board-header", &[]));
    let rows: Vec<&Row> = board
        .rows()
        .iter()
        .filter(|row| &row.key != viewer)
        .collect();
    if rows.is_empty() {
        lines.push(format!("- {}", text(lang, "digest-no-others", &[])));
    } else {
        for row in rows {
            lines.push(format!("- {}", board_line(row, lang)));
        }
    }
    lines.push(String::new());
    lines.push(text(
        lang,
        "digest-rules-line",
        &[("rules", Arg::from(text(lang, "digest-rules", &[])))],
    ));
    lines.push(String::new());
    lines.push(text(lang, "digest-first", &[("root", Arg::from(root))]));
    lines.join("\n")
}

fn board_line(row: &Row, lang: Lang) -> String {
    let name = Arg::from(row.name.as_str());
    if row.doing.is_empty() {
        text(
            lang,
            "digest-board-line",
            &[
                ("name", name),
                ("status", Arg::from(row.status.label(lang))),
            ],
        )
    } else {
        text(
            lang,
            "digest-board-line-doing",
            &[
                ("name", name),
                ("status", Arg::from(row.status.label(lang))),
                ("doing", Arg::from(row.doing.as_str())),
            ],
        )
    }
}

/// 主对话里那行提示：“话题「整理照片目录」：已完成，改了 3 个文件”。
pub(crate) fn finish_notice(name: &str, files: usize, lang: Lang) -> String {
    let name = Arg::from(name);
    if files == 0 {
        text(lang, "digest-done", &[("name", name)])
    } else {
        text(
            lang,
            "digest-done-files",
            &[("name", name), ("files", Arg::from(files.to_string()))],
        )
    }
}

/// 一轮做完时记进变化记录的文字：“谁做完了什么、改了哪些文件”。
pub(crate) fn turn_done_text(
    name: &str,
    summary: &[String],
    files: &[String],
    lang: Lang,
) -> String {
    let name = Arg::from(name);
    let mut line = if summary.is_empty() {
        text(lang, "digest-done-own", &[("name", name)])
    } else {
        text(
            lang,
            "digest-done-summary",
            &[("name", name), ("summary", Arg::from(summary.join("；")))],
        )
    };
    if !files.is_empty() {
        line.push_str(&files_clause(files, lang));
    }
    line
}

/// 文件不超过 5 个列出来，超过只写个数。
pub(crate) fn files_clause(files: &[String], lang: Lang) -> String {
    if files.len() > FILE_LIST_MAX {
        text(
            lang,
            "digest-files-count",
            &[("files", Arg::from(files.len().to_string()))],
        )
    } else {
        text(
            lang,
            "digest-files-list",
            &[("files", Arg::from(files.join("、")))],
        )
    }
}

/// execute 不记文件，只在工作板上记一笔。
pub(crate) fn executed_text(name: &str, title: &str, lang: Lang) -> String {
    text(
        lang,
        "digest-executed",
        &[("name", Arg::from(name)), ("title", Arg::from(title))],
    )
}

/// 两个会话先后改了同一个文件（事后发现）。
pub(crate) fn conflict_text(first: &str, second: &str, path: &str, lang: Lang) -> String {
    text(
        lang,
        "digest-conflict",
        &[
            ("first", Arg::from(first)),
            ("second", Arg::from(second)),
            ("path", Arg::from(path)),
        ],
    )
}

pub(crate) fn ended_text(name: &str, reason: &str, lang: Lang) -> String {
    text(
        lang,
        "digest-ended",
        &[("name", Arg::from(name)), ("reason", Arg::from(reason))],
    )
}

/// 按字符截短，超长以 … 结尾。
pub(crate) fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

/// 名字用：正好取前 max 个字符，不加省略号（ADR-0058：前 20 个字符）。
pub(crate) fn clip_plain(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
