//! 聊天里的停止控件（[`RoomAgentStop`]）：按链列表和停的请求的进展算一条时间线该画什么，
//! [`AgentAction::StopTurns`](wonton_protocol::AgentAction::StopTurns) 把这条时间线里在跑的链都停掉。
//!
//! 链列表是核心的账；停的请求的进展只有这台设备知道，另记在 [`Attempts`] 里。两样合起来才是控件的状态：
//! 列表说没有在跑的，失败和不确定就不再显示（要停的东西已经没了）；列表连不上时留着上次的，那时点了没送到，
//! 显示的就是“不确定”。

use std::collections::HashMap;
use std::time::Duration;

use futures_util::future::join_all;
use tokio::sync::watch;
use url::Url;
use wonton_protocol::{
    AgentAction, AgentChainRow, AgentChainStatus, AgentChainsViewModel, CoreError, ProxySettings,
    RoomAgentStop, TimelineTarget,
};

use super::actions::{Effect, perform};
use super::apply_row;

/// “已停止”留多久：够看清一眼。之后 Bot 的状态行说它怎么收的尾，这里不再占着输入栏。
pub(crate) const STOPPED_SHOWN: Duration = Duration::from_secs(4);

/// 一条时间线上最近一次停的请求走到哪了。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Attempt {
    Stopping,
    Stopped,
    Failed(CoreError),
    Uncertain(CoreError),
}

/// 每条时间线最近一次停的请求：键是房间和旁聊根。
pub(crate) type Attempts = HashMap<(String, Option<String>), Attempt>;

fn key(target: &TimelineTarget) -> (String, Option<String>) {
    (target.room_id.clone(), target.thread_root.clone())
}

/// 从这条时间线开始、本人能停、还有轮次在跑的链。等上限卡片的链也算：轮次挂在那里，停了才放手。
fn live_chains<'a>(
    view: &'a AgentChainsViewModel,
    target: &'a TimelineTarget,
) -> impl Iterator<Item = &'a AgentChainRow> {
    view.chains.iter().filter(move |row| {
        row.live
            && row.can_stop
            && matches!(
                row.status,
                AgentChainStatus::Running | AgentChainStatus::LimitReached
            )
            && row.origin.room_id == target.room_id
            && row.origin.thread_id == target.thread_root
    })
}

/// 这条时间线的停止控件。
pub(crate) fn derive(
    view: &AgentChainsViewModel,
    attempts: &Attempts,
    target: &TimelineTarget,
) -> RoomAgentStop {
    let live = live_chains(view, target).next().is_some();
    match (attempts.get(&key(target)), live) {
        (Some(Attempt::Stopping), _) => RoomAgentStop::Stopping,
        // 停了之后又开了一轮：新的一轮能停，上一次的结果让位。
        (Some(Attempt::Stopped) | None, true) => RoomAgentStop::Running,
        (Some(Attempt::Stopped), false) => RoomAgentStop::Stopped,
        (Some(Attempt::Failed(error)), true) => RoomAgentStop::Failed {
            error: error.clone(),
        },
        (Some(Attempt::Uncertain(error)), true) => RoomAgentStop::Uncertain {
            error: error.clone(),
        },
        (Some(Attempt::Failed(_) | Attempt::Uncertain(_)) | None, false) => RoomAgentStop::Idle,
    }
}

/// 一批停链请求的结果合成一个：有一条没送到就是不确定，否则有一条被拒就是失败，都成了才算停了。
fn settle(results: Vec<Result<Effect, CoreError>>) -> (Vec<AgentChainRow>, Attempt) {
    let mut rows = Vec::new();
    let mut failed = None;
    let mut uncertain = None;
    for result in results {
        match result {
            Ok(Effect::Stopped(row)) => rows.push(row),
            Ok(Effect::Done) => {}
            Err(error @ CoreError::TransportUnavailable { .. }) => {
                uncertain.get_or_insert(error);
            }
            Err(error) => {
                failed.get_or_insert(error);
            }
        }
    }
    let attempt = match (uncertain, failed) {
        (Some(error), _) => Attempt::Uncertain(error),
        (None, Some(error)) => Attempt::Failed(error),
        (None, None) => Attempt::Stopped,
    };
    (rows, attempt)
}

/// 要停的链：这条时间线里在跑的那些，同时记下“在停”。没有在跑的（刚好做完）、已经在停的返回 `None`，不重复发。
pub(crate) fn begin(
    view: &watch::Sender<AgentChainsViewModel>,
    attempts: &watch::Sender<Attempts>,
    target: &TimelineTarget,
) -> Option<Vec<String>> {
    if attempts.borrow().get(&key(target)) == Some(&Attempt::Stopping) {
        return None;
    }
    let chains: Vec<String> = live_chains(&view.borrow(), target)
        .map(|row| row.chain.clone())
        .collect();
    if chains.is_empty() {
        return None;
    }
    record(attempts, target, Attempt::Stopping);
    Some(chains)
}

/// 记下这条时间线上停的请求走到哪了。
pub(crate) fn record(
    attempts: &watch::Sender<Attempts>,
    target: &TimelineTarget,
    attempt: Attempt,
) {
    attempts.send_modify(|attempts| {
        attempts.insert(key(target), attempt);
    });
}

/// 发停链请求、记结果；停成了留 [`STOPPED_SHOWN`] 再清掉。
pub(crate) async fn run(
    view: watch::Sender<AgentChainsViewModel>,
    attempts: watch::Sender<Attempts>,
    connection: (Url, ProxySettings),
    target: TimelineTarget,
    chains: Vec<String>,
) {
    let (core_url, proxy) = &connection;
    let results = join_all(chains.into_iter().map(|chain| async move {
        perform(core_url, proxy, &AgentAction::StopChain { chain }).await
    }))
    .await;
    let (rows, attempt) = settle(results);
    let stopped = attempt == Attempt::Stopped;
    // 先换列表再记结果：订阅的人不会在两步之间看到“停了”的链还在跑。
    view.send_modify(|model| {
        for row in rows {
            apply_row(model, row);
        }
    });
    record(&attempts, &target, attempt);
    if stopped {
        tokio::time::sleep(STOPPED_SHOWN).await;
        expire_stopped(&attempts, &target);
    }
}

/// “已停止”留够了：收走。期间又点过一次的不动。
pub(crate) fn expire_stopped(attempts: &watch::Sender<Attempts>, target: &TimelineTarget) {
    let key = key(target);
    attempts.send_if_modified(|attempts| {
        let shown = attempts.get(&key) == Some(&Attempt::Stopped);
        if shown {
            attempts.remove(&key);
        }
        shown
    });
}

/// 打开时间线时不带上一次的结果：留着的失败对不上现在在跑的这一轮。在停的不动，请求还没回。
pub(crate) fn forget_settled(attempts: &watch::Sender<Attempts>, target: &TimelineTarget) {
    let key = key(target);
    attempts.send_if_modified(|attempts| {
        let settled = attempts
            .get(&key)
            .is_some_and(|attempt| *attempt != Attempt::Stopping);
        if settled {
            attempts.remove(&key);
        }
        settled
    });
}

#[cfg(test)]
#[path = "room_stop_tests.rs"]
mod tests;
