//! 出箱（ADR-0061 第 9、10 节）：核心把要前端（桥）做的事排进这个 Bot 的队列，桥长轮询
//! `GET /agent/outbox` 取走，做完 `POST` 回报。这一块只产出取消轮次（[`OutboxKind::CancelTurn`]），
//! 批准的消息留给第 7 块。
//!
//! 取走即离开队列（`taken` 记着等回报）：另一次轮询不会再拿到同一项。回报失败不重排——重发与否由回报方决定。
//! 队列只在内存里：核心重启时所有连接都断，链也都转成 `Ended`，要取消的轮次随之失效。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use axum::Json;
use axum::body::to_bytes;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::{Method, StatusCode};
use tokio::sync::watch;
use wonton_protocol::{
    AGENT_OUTBOX_WAIT_SECS, BotRef, OutboxItem, OutboxKind, OutboxReceipt, OutboxResult,
};

use super::api::error_response;
use crate::mcp::Caller;

/// 回报体的上限：一项就是 ID 和一个结果，再大是调用方的错。
const MAX_BODY: usize = 16 * 1024;

#[derive(Default)]
struct State {
    queued: BTreeMap<BotRef, VecDeque<OutboxItem>>,
    /// 已取走、等回报的项。
    taken: HashMap<String, OutboxItem>,
    next: u64,
}

/// 出箱：每个 Bot 一条队列，长轮询挂在 `revision` 上。
#[derive(Default)]
pub struct Outbox {
    state: Mutex<State>,
    revision: watch::Sender<u64>,
}

impl Outbox {
    /// 给这个 Bot 排一项，返回项的 ID。别的 Bot 取不走它。
    pub fn push(&self, bot: &BotRef, kind: OutboxKind) -> String {
        let mut state = self.state.lock().expect("出箱锁中毒");
        state.next += 1;
        let id = format!("outbox-{}", state.next);
        state
            .queued
            .entry(bot.clone())
            .or_default()
            .push_back(OutboxItem {
                id: id.clone(),
                kind,
            });
        self.bump();
        id
    }

    /// 取这个 Bot 队首的一项；没有了是 `None`。取走的项进 `taken` 等回报。
    fn take(&self, bot: &BotRef) -> Option<OutboxItem> {
        let mut state = self.state.lock().expect("出箱锁中毒");
        let queue = state.queued.get_mut(bot)?;
        let item = queue.pop_front()?;
        if queue.is_empty() {
            state.queued.remove(bot);
        }
        state.taken.insert(item.id.clone(), item.clone());
        Some(item)
    }

    /// 回报一项：只认取走过的项，重复回报按没做过处理（返回假）。
    fn receipt(&self, receipt: &OutboxReceipt) -> bool {
        let mut state = self.state.lock().expect("出箱锁中毒");
        let Some(item) = state.taken.remove(&receipt.id) else {
            return false;
        };
        match receipt.result {
            OutboxResult::Sent { .. } => {}
            OutboxResult::Failed { category } => {
                tracing::warn!(id = %item.id, ?category, "前端没做成出箱项");
            }
        }
        true
    }

    /// 长轮询：队里有项立即给；没有就挂到有变化或者 [`AGENT_OUTBOX_WAIT_SECS`] 秒。
    /// 先订阅再看队列，中间进来的项不会漏掉。
    async fn next(&self, bot: &BotRef) -> Option<OutboxItem> {
        let mut watcher = self.revision.subscribe();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(AGENT_OUTBOX_WAIT_SECS);
        loop {
            if let Some(item) = self.take(bot) {
                return Some(item);
            }
            match tokio::time::timeout_at(deadline, watcher.changed()).await {
                Ok(Ok(())) => {
                    watcher.borrow_and_update();
                }
                _ => return None,
            }
        }
    }

    fn bump(&self) {
        self.revision.send_modify(|revision| *revision += 1);
    }
}

/// `GET /agent/outbox`：队里有项就给一项，没有就挂到 [`AGENT_OUTBOX_WAIT_SECS`] 秒，超时给 `null`。
pub(super) async fn poll(outbox: &Outbox, caller: &Caller, req: Request) -> Response {
    let Some(bot) = bot_of(caller) else {
        return denied("这个插件没有 Bot 记录，出箱只管有 Bot 的前端");
    };
    if req.method() != Method::GET {
        return error_response(StatusCode::METHOD_NOT_ALLOWED, "invalid_input", "只收 GET");
    }
    Json(outbox.next(&bot).await).into_response()
}

/// `POST /agent/outbox`：回报取走的一项。
pub(super) async fn receipt(outbox: &Outbox, caller: &Caller, req: Request) -> Response {
    if bot_of(caller).is_none() {
        return denied("这个插件没有 Bot 记录，出箱只管有 Bot 的前端");
    }
    if req.method() != Method::POST {
        return error_response(StatusCode::METHOD_NOT_ALLOWED, "invalid_input", "只收 POST");
    }
    let body = match to_bytes(req.into_body(), MAX_BODY).await {
        Ok(body) => body,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体读不出来：{e}"),
            );
        }
    };
    let receipt: OutboxReceipt = match serde_json::from_slice(&body) {
        Ok(receipt) => receipt,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体不是回报：{e}"),
            );
        }
    };
    if !outbox.receipt(&receipt) {
        return error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &format!("没有取走中的出箱项 {}", receipt.id),
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

fn bot_of(caller: &Caller) -> Option<BotRef> {
    caller.bot.as_ref().map(|bot| BotRef {
        plugin_id: caller.id.to_string(),
        bot: bot.bot.clone(),
    })
}

fn denied(message: &str) -> Response {
    error_response(StatusCode::FORBIDDEN, "permission_denied", message)
}
