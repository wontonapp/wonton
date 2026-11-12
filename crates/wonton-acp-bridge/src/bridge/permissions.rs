//! 改文件权限请求的排队卡片（ADR-0058）：另一个会话正在改同一文件时弹三个选项，
//! 默认“等它做完再改”——先不回答 Agent，等对方这一轮结束后自动批准。
//!
//! 卡片本身仍走核心的确认接口：合成三个 choice，再映射回 ACP 原来的允许/拒绝选项；
//! 拒绝时把理由随应答的 `_meta.reason` 递给 Agent。没有冲突就照常只转 Agent 自己的选项。

use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use tokio::sync::oneshot;
use wonton_l10n::{Arg, Lang};

use super::{Room, RoomState, TurnContext, turns};
use crate::acp::PermissionAsk;
use crate::error::Result;
use crate::l10n::text;
use crate::permission::{PendingPermission, PermissionResult};
use crate::topics::{SessionKey, Topics};
use wonton_protocol::{AgentPermissionKind, AgentPermissionOption};

/// 卡片里的“等着”标记；和 ACP 的 optionId 不会撞。
const WAIT: &str = "__wait";
/// 拒绝时 Agent 没给拒绝选项的标记。
const REJECT: &str = "__reject";

/// 桥挂在核心确认接口上的权限长轮询（ADR-0061 第 10 节）：发 `session/cancel` 一方
/// 要把未决的权限请求都回 `cancelled`，所以会话被取消（停链的 `CancelTurn`）时把
/// 它们一并断掉，长轮询随之断开，核心撤回对应的卡片。
/// 会话 → 挂着的取消通道（编号去重，任务结束时摘掉自己）。
type AskCancels = HashMap<u64, oneshot::Sender<()>>;

#[derive(Clone, Default)]
pub(crate) struct PendingAsks {
    asks: Arc<Mutex<HashMap<String, AskCancels>>>,
    next_id: Arc<AtomicU64>,
}

impl PendingAsks {
    /// 登记一个刚挂出去的长轮询，返回它的编号和取消通道；任务结束时凭编号摘掉自己，
    /// 会话被取消时走通道把它断掉。
    fn register(&self, session: &str) -> (u64, oneshot::Receiver<()>) {
        let (cancel, cancelled) = oneshot::channel();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.asks
            .lock()
            .expect("权限长轮询表锁中毒")
            .entry(session.to_owned())
            .or_default()
            .insert(id, cancel);
        (id, cancelled)
    }

    fn unregister(&self, session: &str, id: u64) {
        if let Some(asks) = self
            .asks
            .lock()
            .expect("权限长轮询表锁中毒")
            .get_mut(session)
        {
            asks.remove(&id);
        }
    }

    /// 会话被取消：断掉它挂着的全部长轮询（已结束的通道发不进去，是空操作）。
    pub(crate) fn cancel_session(&self, session: &str) {
        let asks = self
            .asks
            .lock()
            .expect("权限长轮询表锁中毒")
            .remove(session);
        if let Some(asks) = asks {
            for (_, cancel) in asks {
                let _ = cancel.send(());
            }
        }
    }
}

/// 等对方做完的那张卡有了答案，回到循环里处理。
pub(super) struct CardDone {
    key: SessionKey,
    conflict: Option<Conflict>,
    allow: Option<String>,
    reject: Option<String>,
    result: Result<PermissionResult>,
    reply: oneshot::Sender<Result<PermissionResult>>,
}

struct Conflict {
    other: SessionKey,
    path: String,
    other_name: String,
}

/// 等对方这一轮结束才回答的请求。
pub(super) struct ApprovalWaiter {
    key: SessionKey,
    other: SessionKey,
    path: String,
    allow: Option<String>,
    reply: oneshot::Sender<Result<PermissionResult>>,
}

pub(super) async fn handle<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    ask: PermissionAsk,
) -> Result<()> {
    let Some(key) = state.sessions.key_of(&ask.session_id) else {
        spawn_direct(context, ask);
        return Ok(());
    };
    // 权限请求变成核心卡片：这一轮的状态消息编辑成“等你确认”（ADR-0082）。
    state.run_status.waiting(context.room.as_ref(), &key).await;
    // 任务话题的卡片标题写明是哪个任务。
    let mut ask = ask;
    if topics.is_task(&key) {
        let name = topics.name(&key);
        if !name.is_empty() {
            ask.pending.request.title = text(
                context.lang,
                "task-card-title",
                &[
                    ("name", Arg::from(name)),
                    ("title", Arg::from(ask.pending.request.title)),
                ],
            );
        }
    }
    let paths = edit_paths(&ask.pending);
    // 权限请求本身带了路径就先记下来：有的适配层不发带 locations 的工具调用通知。
    topics.claim(&key, &paths, super::now()).await?;
    let conflict = if paths.is_empty() || topics.is_task(&key) {
        None
    } else {
        topics
            .permission_conflict(&key, &paths, super::now())
            .await?
    };
    let (allow, reject) = pick(&ask.pending);
    let Some(other) = conflict else {
        spawn_ask(context, key, ask.pending, None, allow, reject, ask.reply);
        return Ok(());
    };
    let mut pending = ask.pending;
    let other_name = topics.name(&other);
    let path = paths.first().cloned().unwrap_or_default();
    describe(
        &mut pending,
        &allow,
        &reject,
        &other_name,
        &path,
        context.lang,
    );
    spawn_ask(
        context,
        key,
        pending,
        Some(Conflict {
            other,
            path,
            other_name,
        }),
        allow,
        reject,
        ask.reply,
    );
    Ok(())
}

/// 会话不在本房间的表里时直接问卡片，结果照样送回 ACP。
fn spawn_direct<R: Room>(context: &TurnContext<R>, ask: PermissionAsk) {
    let broker = context.bridge.acp.broker();
    let asks = context.bridge.pending_asks.clone();
    let session = ask.session_id.clone();
    let (id, cancel_rx) = asks.register(&session);
    tokio::spawn(async move {
        let result = tokio::select! {
            result = broker.ask(ask.pending) => result,
            _ = cancel_rx => Ok(PermissionResult::Cancelled),
        };
        asks.unregister(&session, id);
        let _ = ask.reply.send(result);
    });
}

fn spawn_ask<R: Room>(
    context: &TurnContext<R>,
    key: SessionKey,
    pending: PendingPermission,
    conflict: Option<Conflict>,
    allow: Option<String>,
    reject: Option<String>,
    reply: oneshot::Sender<Result<PermissionResult>>,
) {
    let broker = context.bridge.acp.broker();
    let cards_tx = context.cards_tx.clone();
    let asks = context.bridge.pending_asks.clone();
    // 请求里带着的会话 ID（acp 层的 handle_request 填）就是这座桥登记取消用的键；
    // 它是本 crate 内部的不变量，缺了是 bug，宁可这一轮挂掉也不静默漏登记。
    let session = pending
        .request
        .session
        .clone()
        .expect("权限请求都带会话 ID");
    let (id, cancel_rx) = asks.register(&session);
    tokio::spawn(async move {
        let result = tokio::select! {
            result = broker.ask(pending) => result,
            _ = cancel_rx => Ok(PermissionResult::Cancelled),
        };
        asks.unregister(&session, id);
        let _ = cards_tx
            .send(CardDone {
                key,
                conflict,
                allow,
                reject,
                result,
                reply,
            })
            .await;
    });
}

/// 卡片有了答案：直接回答、等对方做完、或拒绝并把理由递过去。
pub(super) async fn settle<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    done: CardDone,
) -> Result<()> {
    let key = done.key.clone();
    match done.result {
        Err(error) => {
            let _ = done.reply.send(Err(error));
        }
        Ok(PermissionResult::Selected {
            option_id,
            reason: _,
        }) if option_id == WAIT => {
            let running = done.conflict.as_ref().is_some_and(|conflict| {
                state
                    .sessions
                    .existing(&conflict.other)
                    .is_some_and(|session| session.active)
            });
            tracing::info!(
                running,
                other = done
                    .conflict
                    .as_ref()
                    .map(|c| c.other.storage_key().to_owned()),
                known = done
                    .conflict
                    .as_ref()
                    .is_some_and(|c| state.sessions.existing(&c.other).is_some()),
                "改同一文件的等待选项有了答案"
            );
            if let Some(conflict) = &done.conflict
                && running
            {
                state.waiters.push(ApprovalWaiter {
                    key: key.clone(),
                    other: conflict.other.clone(),
                    path: conflict.path.clone(),
                    allow: done.allow,
                    reply: done.reply,
                });
                return Ok(());
            }
            // 没有可等的人：按“现在就改”回答，聊天里同样写一句已批准。
            let answer = done
                .allow
                .map(PermissionResult::selected)
                .unwrap_or(PermissionResult::Cancelled);
            let _ = done.reply.send(Ok(answer));
            state.run_status.resumed(context.room.as_ref(), &key).await;
            if let Some(conflict) = &done.conflict {
                approve_notice(context, &key, &conflict.other_name, &conflict.path).await;
            }
        }
        Ok(PermissionResult::Selected { option_id, reason }) if option_id == REJECT => {
            let answer = match done.reject {
                Some(reject) => PermissionResult::Selected {
                    option_id: reject,
                    reason: Some(
                        reason.unwrap_or_else(|| reject_reason(&done.conflict, context.lang)),
                    ),
                },
                None => PermissionResult::Cancelled,
            };
            let _ = done.reply.send(Ok(answer));
            state.run_status.resumed(context.room.as_ref(), &key).await;
        }
        Ok(other) => {
            let _ = done.reply.send(Ok(other));
            state.run_status.resumed(context.room.as_ref(), &key).await;
        }
    }
    let _ = topics.permission_settled(&key).await;
    Ok(())
}

fn reject_reason(conflict: &Option<Conflict>, lang: Lang) -> String {
    match conflict {
        Some(conflict) => text(
            lang,
            "conflict-reject-reason",
            &[
                ("other", Arg::from(conflict.other_name.as_str())),
                ("path", Arg::from(conflict.path.as_str())),
            ],
        ),
        None => text(lang, "conflict-reject-reason-short", &[]),
    }
}

/// 对方这一轮结束了：等它的人全部自动批准。
pub(super) async fn release<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    finished: &SessionKey,
) -> Result<()> {
    let mut index = 0;
    while index < state.waiters.len() {
        if &state.waiters[index].other != finished {
            index += 1;
            continue;
        }
        let waiter = state.waiters.remove(index);
        tracing::info!(
            other = finished.storage_key(),
            "对方这一轮结束，自动批准等待的改文件"
        );
        let answer = waiter
            .allow
            .clone()
            .map(PermissionResult::selected)
            .unwrap_or(PermissionResult::Cancelled);
        let _ = waiter.reply.send(Ok(answer));
        let _ = topics.permission_settled(&waiter.key).await;
        state
            .run_status
            .resumed(context.room.as_ref(), &waiter.key)
            .await;
        let name = topics.name(finished);
        approve_notice(context, &waiter.key, &name, &waiter.path).await;
    }
    Ok(())
}

/// 在等的那句批下来时，往等的那条话题里写一句谁做完、批准改哪个文件。
async fn approve_notice<R: Room>(
    context: &TurnContext<R>,
    to: &SessionKey,
    name: &str,
    path: &str,
) {
    let line = text(
        context.lang,
        "conflict-approved",
        &[("name", Arg::from(name)), ("path", Arg::from(path))],
    );
    let _ = turns::send_to(context, to, &line).await;
}

/// 等着的会话自己结束了（取消、超时）：丢掉它的等待。
pub(super) fn drop_waiting(state: &mut RoomState, key: &SessionKey) {
    state.waiters.retain(|waiter| &waiter.key != key);
}

/// 只有编辑、删除、移动类工具、并且带了路径才查冲突。
fn edit_paths(pending: &PendingPermission) -> Vec<String> {
    if !matches!(pending.kind.as_deref(), Some("edit" | "delete" | "move")) {
        return Vec::new();
    }
    pending.locations.clone()
}

/// Agent 给的选项里第一个允许类和第一个拒绝类。
fn pick(pending: &PendingPermission) -> (Option<String>, Option<String>) {
    let mut allow = None;
    let mut reject = None;
    for option in &pending.request.options {
        let mapped = pending.option_ids.get(&option.choice_id).cloned();
        match option.kind {
            AgentPermissionKind::AllowOnce | AgentPermissionKind::AllowAlways
                if allow.is_none() =>
            {
                allow = mapped
            }
            AgentPermissionKind::RejectOnce | AgentPermissionKind::RejectAlways
                if reject.is_none() =>
            {
                reject = mapped
            }
            _ => {}
        }
    }
    (allow, reject)
}

/// 把卡片改成写明谁在改哪个文件，换成三个选项（默认这项排第一）。
fn describe(
    pending: &mut PendingPermission,
    allow: &Option<String>,
    _reject: &Option<String>,
    other_name: &str,
    path: &str,
    lang: Lang,
) {
    let other = Arg::from(other_name);
    let path_arg = Arg::from(path);
    let note = text(
        lang,
        "conflict-in-use-note",
        &[("other", other), ("path", path_arg)],
    );
    pending.request.title = text(
        lang,
        "conflict-in-use-title",
        &[
            ("title", Arg::from(pending.request.title.as_str())),
            ("other", Arg::from(other_name)),
        ],
    );
    pending.request.description = Some(match pending.request.description.take() {
        Some(description) => format!("{note}\n{description}"),
        None => note,
    });
    pending.request.options = vec![
        AgentPermissionOption {
            choice_id: WAIT.into(),
            kind: AgentPermissionKind::AllowOnce,
            name: text(
                lang,
                "conflict-wait-option",
                &[("other", Arg::from(other_name))],
            ),
        },
        AgentPermissionOption {
            choice_id: "now".into(),
            kind: AgentPermissionKind::AllowOnce,
            name: text(lang, "conflict-now-option", &[]),
        },
        AgentPermissionOption {
            choice_id: "reject".into(),
            kind: AgentPermissionKind::RejectOnce,
            name: text(lang, "conflict-reject-option", &[]),
        },
    ];
    pending.locations = vec![path.to_owned()];
    pending.option_ids = HashMap::from([
        (WAIT.to_owned(), WAIT.to_owned()),
        (
            "now".to_owned(),
            allow.clone().unwrap_or_else(|| REJECT.to_owned()),
        ),
        ("reject".to_owned(), REJECT.to_owned()),
    ]);
}
