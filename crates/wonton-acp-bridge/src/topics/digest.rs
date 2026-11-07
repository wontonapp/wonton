//! 动态摘要、话题开局说明和 notice 的文字。这些文字给 Agent 和聊天看，
//! 中文常量先写在这里；按 ADR-0043 做语言化不在这一步。

use crate::topics::SessionKey;
use crate::topics::board::{Board, NAME_MAX_CHARS, Row, Status};

const DIGEST_HEADER: &str = "其他会话的动态：";
/// 动态里最多列最近几条变化（ADR-0058：内容要截短）。
const DIGEST_MAX_CHANGES: usize = 5;
/// 摘要和 notice 里文件超过几个只写个数（ADR-0058）。
const FILE_LIST_MAX: usize = 5;
/// 话题开局带话题起点之前的主对话消息条数。
pub(crate) const OPENING_CONTEXT_MESSAGES: usize = 6;
/// 话题开局里每条片段的截短长度。
const CONTEXT_MAX_CHARS: usize = 120;
/// ADR-0058 的分工规矩原话。
const RULES: &str = "只负责这个话题里用户要的事；主对话和其他话题在做的事不要接着做；要碰它们正在改的文件，先停下问用户。";

/// 其他会话从 viewer 上一轮开始到现在的工作板变化；没有变化返回 `None`。
pub(crate) fn digest(board: &Board, viewer: &SessionKey) -> Option<String> {
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
        .filter_map(|row| live_line(row).map(|line| format!("- {line}")))
        .collect();
    if changes.is_empty() && live.is_empty() {
        return None;
    }
    let mut lines = vec![DIGEST_HEADER.to_owned()];
    lines.extend(changes);
    lines.extend(live);
    Some(lines.join("\n"))
}

/// 还在做事的会话在动态里的一行：“现在在做什么”。
fn live_line(row: &Row) -> Option<String> {
    match row.status {
        Status::Working => Some(format!("「{}」正在做：{}", row.name, row.doing)),
        Status::WaitingApproval => Some(format!("「{}」等你确认操作", row.name)),
        Status::Queued => Some(format!("「{}」排队中", row.name)),
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
) -> String {
    let name = clip_plain(root, NAME_MAX_CHARS);
    let mut lines = vec![format!("这是从主对话分出来的话题「{name}」。")];
    lines.push("主对话的片段（仅供参考）：".to_owned());
    let start = preceding.len().saturating_sub(OPENING_CONTEXT_MESSAGES);
    for message in &preceding[start..] {
        lines.push(format!("- {}", clip(message, CONTEXT_MAX_CHARS)));
    }
    lines.push(format!("- {}", clip(root, CONTEXT_MAX_CHARS)));
    lines.push(String::new());
    lines.push("当前工作板：".to_owned());
    let rows: Vec<&Row> = board
        .rows()
        .iter()
        .filter(|row| &row.key != viewer)
        .collect();
    if rows.is_empty() {
        lines.push("- 暂无其他会话".to_owned());
    } else {
        for row in rows {
            lines.push(format!("- {}", board_line(row)));
        }
    }
    lines.push(String::new());
    lines.push(format!("分工规矩：{RULES}"));
    lines.push(String::new());
    lines.push(format!("用户的第一句：{root}"));
    lines.join("\n")
}

fn board_line(row: &Row) -> String {
    if row.doing.is_empty() {
        format!("「{}」{}", row.name, row.status.label())
    } else {
        format!("「{}」{}：{}", row.name, row.status.label(), row.doing)
    }
}

/// 主对话里那行提示：“话题「整理照片目录」：已完成，改了 3 个文件”。
pub(crate) fn finish_notice(name: &str, files: usize) -> String {
    if files == 0 {
        format!("话题「{name}」：已完成")
    } else {
        format!("话题「{name}」：已完成，改了 {files} 个文件")
    }
}

/// 一轮做完时记进变化记录的文字：“谁做完了什么、改了哪些文件”。
pub(crate) fn turn_done_text(name: &str, summary: &[String], files: &[String]) -> String {
    let mut text = if summary.is_empty() {
        format!("「{name}」已完成")
    } else {
        format!("「{name}」已完成：{}", summary.join("；"))
    };
    if !files.is_empty() {
        text.push_str(&files_clause(files));
    }
    text
}

/// 文件不超过 5 个列出来，超过只写个数。
pub(crate) fn files_clause(files: &[String]) -> String {
    if files.len() > FILE_LIST_MAX {
        format!("，改了 {} 个文件", files.len())
    } else {
        format!("，改了：{}", files.join("、"))
    }
}

/// execute 不记文件，只在工作板上记一笔。
pub(crate) fn executed_text(name: &str, title: &str) -> String {
    format!("「{name}」执行了「{title}」")
}

/// 两个会话先后改了同一个文件（事后发现）。
pub(crate) fn conflict_text(first: &str, second: &str, path: &str) -> String {
    format!("「{first}」和「{second}」都改了 {path}（事后发现）")
}

pub(crate) fn ended_text(name: &str, reason: &str) -> String {
    format!("「{name}」已结束（{reason}）")
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
