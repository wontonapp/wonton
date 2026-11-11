//! ACP 通知：命令菜单跟着 Agent 走，工具调用记进改文件表，事后发现的冲突两边各说一句。

use std::result::Result as StdResult;

use tokio::sync::broadcast;

use wonton_l10n::Arg;

use super::{Room, RoomState, TurnContext, now, turns};
use crate::acp::AgentNotice;
use crate::commands::Commands;
use crate::error::Result;
use crate::l10n::text;
use crate::topics::Topics;

pub(super) async fn handle_notice<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    commands: &mut Commands,
    notice: StdResult<AgentNotice, broadcast::error::RecvError>,
) -> Result<()> {
    let notice = match notice {
        Ok(notice) => notice,
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            tracing::warn!(skipped, "ACP 通知跟不上，命令菜单可能停在旧表");
            return Ok(());
        }
        Err(broadcast::error::RecvError::Closed) => return Ok(()),
    };
    match notice {
        AgentNotice::Update { session_id, update } => {
            let Some(key) = state.sessions.key_of(&session_id) else {
                return Ok(());
            };
            commands
                .receive(
                    &super::RoomSink {
                        room: context.room.as_ref(),
                        bot: &context.bridge.bot,
                        thread: None,
                    },
                    None,
                    &session_id,
                    &update,
                    context.lang,
                )
                .await;
            report_conflict(context, topics, &key, &update).await;
        }
        AgentNotice::PermissionCancelled { .. }
        | AgentNotice::PermissionFailed { .. }
        | AgentNotice::Task { .. } => {}
    }
    Ok(())
}

async fn report_conflict<R: Room>(
    context: &TurnContext<R>,
    topics: &mut Topics,
    key: &crate::topics::SessionKey,
    update: &serde_json::Value,
) {
    let conflict = match topics.tool_call(key, update, now(), context.lang).await {
        Ok(conflict) => conflict,
        Err(error) => {
            tracing::warn!(error = %error, "记改文件表失败");
            return;
        }
    };
    let Some(conflict) = conflict else {
        return;
    };
    let own_name = topics.name(key);
    let other_name = topics.name(&conflict.other);
    let line = text(
        context.lang,
        "digest-conflict",
        &[
            ("first", Arg::from(own_name)),
            ("second", Arg::from(other_name)),
            ("path", Arg::from(conflict.path.as_str())),
        ],
    );
    let _ = turns::send_to(context, key, &line).await;
    let _ = turns::send_to(context, &conflict.other, &line).await;
}
