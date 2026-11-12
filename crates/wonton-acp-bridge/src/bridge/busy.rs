//! Agent 正在跑一轮时进来的消息（ADR-0035）：只有一种行为。
//!
//! 能插话（`_session/steering`）就插进正在跑的一轮；Agent 不支持、或者没收下（`promptRequired`、`failed`、
//! 请求出错），就攒着，这一轮结束后把攒着的全部合成一轮发出去。给主人的反馈只有已读回执：Agent 真的
//! 收到时（插进去了，或者含这条的一轮发出去了）才对这条消息发回执；攒着不另行提示。

use super::{Job, Room, RoomState, TurnContext, format_error, turns};
use crate::acp::SteerOutcome;
use crate::error::{Detail, Error, Result};
use crate::l10n::text;
use crate::topics::SessionKey;

/// 一条普通消息（文字或附件）在这一轮跑着时进来了。
pub(super) async fn receive<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    key: &SessionKey,
    job: Job,
) -> Result<()> {
    let session = state.sessions.state(key);
    // 已经攒了消息就接着攒：这条插进去会跑到先到的那些前面。正在打断时插进去的会随那一轮一起丢掉。
    let steerable = context.bridge.acp.capabilities().steering
        && session.buffer.is_empty()
        && !session.cancelling;
    let session_id = session.session_id.clone();
    let (true, Some(session_id)) = (steerable, session_id) else {
        return hold(context, state, key, job).await;
    };
    // 插话和发一轮用同样的内容块（ACP 的约定），图片照 `promptCapabilities` 给 image 或 resource_link。
    let blocks = match turns::prompt_blocks(context, None, std::slice::from_ref(&job)).await {
        Ok(blocks) => blocks,
        Err(error) => {
            return turns::send_to(context, key, &format_error(&error, context.lang)).await;
        }
    };
    match context.bridge.acp.steer(&session_id, blocks).await {
        Ok(SteerOutcome::Injected) => {
            receipt(context, &job.event_id).await;
            Ok(())
        }
        Ok(SteerOutcome::StartedNewTurn) => {
            receipt(context, &job.event_id).await;
            turns::send_to(context, key, &text(context.lang, "steer-new-turn", &[])).await
        }
        Ok(SteerOutcome::PromptRequired | SteerOutcome::Failed) => {
            hold(context, state, key, job).await
        }
        Err(error) => {
            tracing::warn!(error = %error, "插话请求出错，这条攒到这一轮结束后发");
            hold(context, state, key, job).await
        }
    }
}

/// `/wonton interrupt <文字>`（调试用）：取消这一轮，文字和攒着的消息在它收尾后合成一轮发。
pub(super) async fn interrupt<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    key: &SessionKey,
    job: Job,
) -> Result<()> {
    let session = state.sessions.state(key);
    session.buffer.push(job);
    let Some(session_id) = session.session_id.clone() else {
        return Ok(());
    };
    session.cancelling = true;
    if let Err(error) = context.bridge.acp.cancel(&session_id).await {
        state.sessions.state(key).cancelling = false;
        return turns::send_to(context, key, &format_error(&error, context.lang)).await;
    }
    Ok(())
}

/// 攒着，等这一轮结束。攒满了如实报忙，这条不收。
async fn hold<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    key: &SessionKey,
    job: Job,
) -> Result<()> {
    let session = state.sessions.state(key);
    if session.buffer.len() >= context.bridge.config.limits.max_queue {
        let error = Error::Busy(Detail::msg("queue-full"));
        return turns::send_to(context, key, &format_error(&error, context.lang)).await;
    }
    session.buffer.push(job);
    Ok(())
}

/// 这一轮结束时取出攒着的消息，交给调用方合成一轮。
pub(super) fn take_held(state: &mut RoomState, key: &SessionKey) -> Vec<Job> {
    let session = state.sessions.state(key);
    session.cancelling = false;
    std::mem::take(&mut session.buffer)
}

/// 对 Agent 已经收到的消息发已读回执。回执只是给主人看的反馈，发不出去不影响这一轮，记日志。
pub(super) async fn receipt<R: Room>(context: &TurnContext<R>, event_id: &str) {
    if let Err(error) = context.room.read_receipt(event_id).await {
        tracing::warn!(error = %error, event_id, "发已读回执失败");
    }
}
