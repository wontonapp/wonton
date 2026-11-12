//! 话题的上限：同时活跃的话题最多 3 个，第 4 个排队；空闲 30 分钟的话题判为结束。

use wonton_l10n::Lang;

use crate::l10n::text;
use crate::topics::SessionKey;
use crate::topics::board::{Board, Status};
use crate::topics::digest;

/// 同时活跃（开着、没结束）的话题上限（ADR-0058：3 个是估计值，按实测调）。
pub(crate) const MAX_ACTIVE_TOPICS: usize = 3;
/// 话题空闲多久判为结束（配置的默认值，也在测试里用）。
#[cfg(test)]
pub(crate) const IDLE_TOPIC_TTL_MS: u64 = 30 * 60 * 1000;

/// 新话题能否直接开始：名额未满，且没有在排队的话题（排队的一定先来，不许插队）。
pub(crate) fn can_admit(board: &Board) -> bool {
    board.active_topics() < MAX_ACTIVE_TOPICS && board.queued_keys().is_empty()
}

/// 名额有空就放最老的一个排队话题进来，返回它的会话键、它的首条消息和那条消息的
/// 事件 ID；名额不够（或没有排队的）返回 `None`。
pub(crate) fn poll_admission(board: &mut Board, now: u64) -> Option<(SessionKey, String, String)> {
    let key = board.queued_keys().into_iter().next()?;
    if board.active_topics() >= MAX_ACTIVE_TOPICS {
        return None;
    }
    let row = board.row_mut(&key)?;
    let prompt = row.pending_prompt.take()?;
    let run = row.pending_run.take().unwrap_or_default();
    row.status = Status::Working;
    row.last_activity = now;
    Some((key, prompt, run))
}

/// 把空闲超过 30 分钟的话题判为结束，返回被结束的会话键（调用方去关 Agent 会话并
/// 在话题里告知）；主对话不参与。
pub(crate) fn sweep(board: &mut Board, now: u64, ttl_ms: u64, lang: Lang) -> Vec<SessionKey> {
    let mut ended = Vec::new();
    for row in board.rows_mut() {
        if row.key.is_topic()
            && row.status != Status::Finished
            && now.saturating_sub(row.last_activity) >= ttl_ms
        {
            row.status = Status::Finished;
            ended.push(row.key.clone());
        }
    }
    for key in &ended {
        let name = board
            .row(key)
            .map(|row| row.name.clone())
            .unwrap_or_default();
        board.log_change(
            key.clone(),
            digest::ended_text(&name, &text(lang, "topic-ended-idle", &[]), lang),
            false,
        );
    }
    ended
}

/// `/wonton end`：把话题标成已结束，结论留在工作板上。主对话和未知会话返回 `false`。
pub(crate) fn end(board: &mut Board, key: &SessionKey, now: u64, lang: Lang) -> bool {
    let Some(row) = board.row_mut(key) else {
        return false;
    };
    if !row.key.is_topic() || row.status == Status::Finished {
        return false;
    }
    row.status = Status::Finished;
    row.last_activity = now;
    let name = row.name.clone();
    let by = key.clone();
    board.log_change(
        by,
        digest::ended_text(&name, &text(lang, "topic-ended-manual", &[]), lang),
        false,
    );
    true
}
