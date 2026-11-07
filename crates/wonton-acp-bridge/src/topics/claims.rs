//! 改文件表：谁正在改哪个文件。一个会话这一轮改过的文件，到这一轮结束后 10 分钟内
//! 都算它在改（ADR-0058）。
//!
//! 时间由调用方传入（Unix 毫秒），模块里不读系统时钟。路径来自 ACP
//! `tool_call`/`tool_call_update` 里 kind 为 `edit`、`delete`、`move` 的
//! `locations[].path`；`execute` 不记文件，只在工作板上记一笔。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::topics::SessionKey;

/// 一轮结束后改文件记录继续算“在改”的时长。
pub(crate) const CLAIM_TTL_MS: u64 = 10 * 60 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Claim {
    pub(crate) by: SessionKey,
    pub(crate) path: String,
    /// 算“在改”的截止时刻（Unix 毫秒）；`None` 表示这一轮还没结束，一直算。
    pub(crate) expires: Option<u64>,
}

/// 一次工具调用里值得记录的内容。
pub(crate) enum ToolEdit {
    /// edit/delete/move 带的文件路径。
    Files(Vec<String>),
    /// execute 命令的标题。
    Executed(String),
    /// 其他类别，或没有 kind 的部分更新（ACP 的 tool_call_update 只带变化字段）。
    Ignored,
}

pub(crate) fn parse_tool_call(update: &Value) -> ToolEdit {
    if !matches!(
        update["sessionUpdate"].as_str(),
        Some("tool_call" | "tool_call_update")
    ) {
        return ToolEdit::Ignored;
    }
    match update["kind"].as_str() {
        Some("edit" | "delete" | "move") => ToolEdit::Files(locations(update)),
        Some("execute") => {
            ToolEdit::Executed(update["title"].as_str().unwrap_or("命令").to_owned())
        }
        _ => ToolEdit::Ignored,
    }
}

fn locations(update: &Value) -> Vec<String> {
    update["locations"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["path"].as_str().filter(|path| !path.is_empty()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn active(claim: &Claim, now: u64) -> bool {
    match claim.expires {
        Some(at) => now < at,
        None => true,
    }
}

/// 记下这个会话在改这些文件（一直算到它的轮结束盖章为止）；顺手清掉已过期的旧记录。
pub(crate) fn record(claims: &mut Vec<Claim>, key: &SessionKey, paths: &[String], now: u64) {
    claims.retain(|claim| active(claim, now));
    for path in paths {
        match claims
            .iter_mut()
            .find(|claim| &claim.by == key && claim.path == *path)
        {
            // 上一轮的同名记录还在保留期，这轮又改：改回“这轮没结束”的口径。
            Some(claim) => claim.expires = None,
            None => claims.push(Claim {
                by: key.clone(),
                path: path.clone(),
                expires: None,
            }),
        }
    }
}

/// 这些路径上是否有别的会话正在改；有就返回第一个命中的（会话，路径）。
pub(crate) fn conflict_on(
    claims: &[Claim],
    key: &SessionKey,
    paths: &[String],
    now: u64,
) -> Option<(SessionKey, String)> {
    for path in paths {
        if let Some(claim) = claims
            .iter()
            .find(|claim| active(claim, now) && &claim.by != key && claim.path == *path)
        {
            return Some((claim.by.clone(), path.clone()));
        }
    }
    None
}

/// 一轮结束：这个会话还没盖章的记录改成“结束后再保留 10 分钟”。
pub(crate) fn seal_turn(claims: &mut [Claim], key: &SessionKey, now: u64) {
    seal_from(claims, |claim| &claim.by == key, now);
}

/// 重启恢复：还没盖章的记录不知道上一轮是否已结束，从现在起给一段保留期。
pub(crate) fn seal_after_load(claims: &mut [Claim], now: u64) {
    seal_from(claims, |_| true, now);
}

fn seal_from(claims: &mut [Claim], matches: impl Fn(&Claim) -> bool, now: u64) {
    let deadline = now + CLAIM_TTL_MS;
    for claim in claims
        .iter_mut()
        .filter(|claim| claim.expires.is_none() && matches(claim))
    {
        claim.expires = Some(deadline);
    }
}
