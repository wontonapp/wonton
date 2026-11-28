//! 给 Agent 看的任务和会话：只留它要的。上传任务的条目可能上千，只报条数和前几条失败；会话不带播放计划，
//! 计划里的流地址是给设备取字节用的，Agent 用不上。

use serde_json::{Value, json};
use wonton_protocol::media::{MediaTask, PlaybackSession, TaskTarget};

/// 上传任务里逐项失败最多列这么多条，多的只报条数。
const FAILED_ITEMS: usize = 5;

pub(super) fn task(task: &MediaTask) -> Value {
    let (items, item) = match &task.target {
        TaskTarget::Upload(items) => (Some(items.len()), None),
        TaskTarget::Download(item) => (None, Some(item)),
        TaskTarget::Other(_) => (None, None),
    };
    let failed: Vec<Value> = task
        .partial_failures
        .iter()
        .take(FAILED_ITEMS)
        .map(|failed| json!({"item_id": failed.item_id, "path": failed.path, "failure": failed.failure}))
        .collect();
    json!({
        "id": task.id,
        "kind": task.kind,
        "device": task.device,
        "state": task.state,
        "progress": task.progress,
        "items": items,
        "item": item,
        "retry_count": task.retry_count,
        "retry_of": task.retry_of,
        "next_attempt_at": task.next_attempt_at,
        "created_at": task.created_at,
        "failed_items": failed,
        "more_failed_items": task.partial_failures.len().saturating_sub(FAILED_ITEMS),
    })
}

pub(super) fn session(session: &PlaybackSession) -> Value {
    json!({
        "id": session.id,
        "device": session.device,
        "item": session.item,
        "state": session.state,
        "position_ms": session.position_ms,
        "stop_reason": session.stop_reason,
    })
}
