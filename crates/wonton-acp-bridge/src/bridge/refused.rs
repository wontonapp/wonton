//! 本 Bot 的消息被网关拒收（ADR-0045 的跳数）。拒收的消息停在发送队列队首，挡住这个房间之后的所有消息，SDK 不重发，
//! 桥不能让它们就这样没了：
//!
//! - 从时间线里自己消息的发送状态认出拒收，按 `errcode` 分类（[`HOP_LIMIT_ERRCODE`]），记日志。主人这时已经从核心的
//!   “已停下”卡片知道了：房间被停下，桥自己发什么都会被拒。
//! - 跳数清零之后重发，按原来的顺序：有人在房间说话（网关清零），或者本 Bot 又有新消息排进队（主人在卡片上选了
//!   “继续”，桥看不到卡片，只能在下一条要发时试一次）。没有定时重试：还没清零时重发一次再被拒，停在那里等下一次。
//! - 补发完在房间里如实说一句：几条消息被停下过、已经补发。
//!
//! 别的拒收（不是跳数）不重发，只记日志：原因没消失，重发也没用。

use std::collections::HashSet;

use wonton_matrix::{Message, SendStatus, UserId};
use wonton_protocol::HOP_LIMIT_ERRCODE;

use super::TurnContext;
use crate::error::{Error, Result, format_error};
use crate::room::Room;

/// 拒收的类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    HopLimit,
    Other,
}

/// 按发送失败的原话分类。matrix-sdk 只留下错误的文字，`errcode` 在里面原样出现。
pub(super) fn classify(error: &str) -> Refusal {
    if error.contains(HOP_LIMIT_ERRCODE) {
        Refusal::HopLimit
    } else {
        Refusal::Other
    }
}

/// 看一批消息之后要做的事。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    Nothing,
    Resend,
    /// 停下过的 `n` 条都发出去了，告诉主人。
    Report(usize),
}

/// 一个房间里本 Bot 被拒收的消息。
#[derive(Default)]
pub(super) struct Refused {
    hop: usize,
    other: usize,
    queued: usize,
    /// 被跳数停下过、还没报给主人的条数。
    delayed: usize,
}

impl Refused {
    /// `someone_spoke`：这批里有名单上的人说了新话。
    pub(super) fn observe(&mut self, messages: &[Message], someone_spoke: bool) -> Step {
        let (mut hop, mut other, mut queued) = (0, 0, 0);
        for message in messages.iter().filter(|message| message.is_own) {
            match &message.status {
                SendStatus::Failed {
                    error,
                    recoverable: false,
                } => match classify(error) {
                    Refusal::HopLimit => hop += 1,
                    Refusal::Other => other += 1,
                },
                SendStatus::Sending { .. } => queued += 1,
                SendStatus::Sent | SendStatus::Failed { .. } => {}
            }
        }
        if hop > self.hop {
            tracing::warn!(
                category = "hop_limit",
                count = hop,
                "几个 Bot 来回太多，网关拒收了本 Bot 的消息，等跳数清零后重发"
            );
        }
        if other > self.other {
            tracing::warn!(count = other, "服务器拒收了本 Bot 的消息，不重发");
        }
        let new_output = queued > self.queued;
        self.delayed = self.delayed.max(hop);
        (self.hop, self.other, self.queued) = (hop, other, queued);
        if hop > 0 && (someone_spoke || new_output) {
            return Step::Resend;
        }
        if hop == 0 && queued == 0 && self.delayed > 0 {
            return Step::Report(std::mem::take(&mut self.delayed));
        }
        Step::Nothing
    }
}

/// 照 [`Refused::observe`] 的结果重发或告诉主人。
pub(super) async fn handle<R: Room>(
    context: &TurnContext<R>,
    refused: &mut Refused,
    messages: &[Message],
    seen: &HashSet<String>,
    owner: &UserId,
) -> Result<()> {
    // 时间线每次给全部消息：只有还没见过的才是新说的话。
    let allowed = &context.bridge.config.allowed_users;
    let someone_spoke = messages.iter().any(|message| {
        !message.is_own
            && allowed.contains(&message.sender)
            && message
                .event_id
                .as_ref()
                .is_some_and(|id| !seen.contains(id.as_str()))
    });
    match refused.observe(messages, someone_spoke) {
        Step::Nothing => Ok(()),
        Step::Resend => {
            if let Err(error) = context.room.resend_refused().await {
                tracing::warn!(error = %error, "重发被拒收的消息失败");
            }
            Ok(())
        }
        Step::Report(count) => {
            let lang = context.bridge.lang(owner).await;
            context
                .room
                .send_notice(&format_error(&Error::HopLimit { count }, lang))
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hop_limit_is_told_apart_from_other_refusals() {
        let hop = format!("[403 / {HOP_LIMIT_ERRCODE}] 几个 Bot 来回说得太多了");
        assert_eq!(classify(&hop), Refusal::HopLimit);
        assert_eq!(classify("[403 / M_FORBIDDEN] no"), Refusal::Other);
    }
}
