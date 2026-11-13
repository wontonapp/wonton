//! 旁聊（ADR-0093）：主对话里的结果行、支持声明和标题。
//!
//! 结果行是主对话里不挂话题的 `m.notice`，带 `app.wonton.side_chat_result` 和指向话题
//! 根的 `m.in_reply_to`；一轮结束发 done（答完了）或 failed（失败或被打断），旁聊结束
//! （空闲清理、`/wonton end`）发 ended。标题只有一条规则（ADR-0093 §1）：根消息带
//! `app.wonton.side_chat.title` 就用它，否则取根正文第一行的前 20 个字符。

use std::collections::HashMap;

use serde_json::{Value, json};

use wonton_l10n::{Arg, Lang};
use wonton_matrix::{SIDE_CHAT_RESULT_FIELD, SIDE_CHAT_SUPPORT_EVENT, UserId};
use wonton_protocol::{SideChatResult, SideChatResultState, SideChatSupport};

use super::run_status;
use crate::acp::PromptResult;
use crate::error::Result;
use crate::l10n::text;
use crate::room::Room;

/// 看过的旁聊根（事件 ID → 标题）：根消息不是输入，进来只记标题，话题第一轮取名字用。
#[derive(Default)]
pub(super) struct Titles(HashMap<String, String>);

impl Titles {
    pub(super) fn observe(&mut self, root: &str, title: String) {
        self.0.insert(root.to_owned(), title);
    }

    pub(super) fn title_of(&self, root: &str) -> Option<&str> {
        self.0.get(root).map(String::as_str)
    }
}

/// 话题的标题（ADR-0093 §1）：根消息带标题字段用它；否则取根正文第一行的前 20 个字符。
/// `root_body` 是根正文；根不在主对话历史里（桥重启过）时是话题的第一句话——App 开旁聊
/// 时标题正是按它取的，取出来一样。
pub(super) fn title_of(titles: &Titles, root: &str, root_body: &str) -> String {
    titles
        .title_of(root)
        .map(str::to_owned)
        .unwrap_or_else(|| wonton_matrix::title_from_body(root_body))
}

/// 这一轮按状态消息（ADR-0082）的口径答完了就发 done，其余发 failed。
pub(super) fn result_state(result: &Result<PromptResult>) -> SideChatResultState {
    if run_status::answered(result) {
        SideChatResultState::Done
    } else {
        SideChatResultState::Failed
    }
}

/// 主对话里发一条结果行。发不出去只记日志：一轮本身已经收尾，缺了这行卡片按
/// `ResultMissing` 显示（ADR-0093 §4），不为此打断房间循环。
pub(super) async fn result_line<R: Room>(
    room: &R,
    root: &str,
    title: &str,
    state: SideChatResultState,
    lang: Lang,
) {
    let key = match state {
        SideChatResultState::Done => "side-chat-result-done",
        SideChatResultState::Failed => "side-chat-result-failed",
        SideChatResultState::Ended => "side-chat-result-ended",
    };
    let body = text(lang, key, &[("title", Arg::from(title))]);
    if let Err(error) = room
        .send_side_chat_result(content(root, title, state, body))
        .await
    {
        tracing::warn!(error = %error, "发旁聊结果行失败");
    }
}

/// 结果行的事件内容（ADR-0093 §1）：话题根放在结构化字段 `root` 里，App 按它跳，
/// 不解析 `body`；`m.in_reply_to` 指向根，让 Element 这类客户端也能点回去。
fn content(root: &str, title: &str, state: SideChatResultState, body: String) -> Value {
    let result = serde_json::to_value(SideChatResult {
        v: 1,
        root: root.to_owned(),
        title: title.to_owned(),
        state,
    })
    .expect("结果行字段只有整数、字符串和枚举，序列化不会失败");
    json!({
        "msgtype": "m.notice",
        "body": body,
        "m.relates_to": { "m.in_reply_to": { "event_id": root } },
        (SIDE_CHAT_RESULT_FIELD): result,
    })
}

/// 在这个房间声明支持旁聊（ADR-0093 §1）：状态事件的 `state_key` 是 Bot 自己的 Matrix ID。
pub(super) async fn declare<R: Room>(room: &R, bot: &UserId) -> Result<()> {
    let support = serde_json::to_value(SideChatSupport { v: 1 })
        .expect("支持声明只有一个整数字段，序列化不会失败");
    room.set_state(SIDE_CHAT_SUPPORT_EVENT, bot.as_str(), support)
        .await
}

#[cfg(test)]
#[path = "side_chats_tests.rs"]
mod tests;
