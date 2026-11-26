//! Siri、快捷指令的“问 Bot”：把一句话发给默认 Agent，并把这次动作的房间和结果写进回执（ADR-0041）。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_l10n::l10n;
use wonton_matrix::{Error, OwnedRoomId, OwnedUserId};
use wonton_protocol::{
    AskFailure, AskProblem, AskReceipt, AskState, CoreError, GatewayRefusal, OwnBotFailure,
    OwnPrivateBotView, PrivateBotRoomProblem, RoomMembership,
};

use super::{ChatLink, onboarding};
use crate::l10n::{CATALOG, language};

/// 壳发起的一次“问 Bot”。
pub(crate) struct AskRequest {
    pub text: String,
    /// 壳当前所在的房间；只有它是呼叫者可用 Bot 的私聊或本人 Bot 的主房间时才用。
    pub room_id: Option<String>,
    /// 壳生成的动作 ID，原样写进回执。
    pub request_id: Option<u32>,
}

/// 当前人的默认 Bot 投影。`Visible` 是引导里已经核实可见的默认 Bot；`Absent`、`Unknown` 分得清“没有”
/// 和“还没核实”。
pub(crate) enum DefaultBot {
    Visible(OwnedUserId),
    Absent,
    Unknown,
}

/// 解析出的目标：加入 `room`（或者从 `bot` 新建私聊），再按 `send` 决定发不发。
struct AskAttempt {
    bot: Option<OwnedUserId>,
    room: Option<OwnedRoomId>,
    join: bool,
}

impl ChatLink {
    /// 发给默认 Bot 的一句话；结果写进 [`ChatViewModel::ask`](wonton_protocol::ChatViewModel::ask)。
    /// `text` 去掉首尾空白后为空时只解析并打开私聊，不发消息（桌面小组件、Agent 入口用它）。
    /// 同步能判出来的失败也写成 `Failed` 回执返回 `Ok`，让壳只有一个来源可看。
    pub(crate) fn ask(
        self: &Arc<Self>,
        request: AskRequest,
        default_bot: DefaultBot,
    ) -> Result<(), CoreError> {
        let seq = self.next_ask_seq();
        let body = request.text.trim().to_owned();
        let attempt = match self.resolve_ask(&request, &default_bot) {
            Ok(attempt) => attempt,
            Err(problem) => {
                self.publish_ask(
                    seq,
                    AskReceipt {
                        request_id: request.request_id,
                        room_id: request.room_id.clone(),
                        state: AskState::Failed,
                        problem: Some(problem),
                    },
                );
                return Ok(());
            }
        };
        let pending_room = attempt.room.clone();
        self.publish_ask(
            seq,
            AskReceipt {
                request_id: request.request_id,
                room_id: pending_room.map(|room| room.to_string()),
                state: AskState::Pending,
                problem: None,
            },
        );
        let chat = self.clone();
        let send = !body.is_empty();
        let request_id = request.request_id;
        self.runtime.spawn(async move {
            let receipt = match chat.deliver_ask(&attempt, &body, send).await {
                Ok(room) => AskReceipt {
                    request_id,
                    room_id: Some(room.to_string()),
                    state: AskState::Sent,
                    problem: None,
                },
                Err((room, error)) => AskReceipt {
                    request_id,
                    room_id: room.map(|room| room.to_string()),
                    state: AskState::Failed,
                    problem: Some(ask_problem(ask_failure(&error), Some(&error))),
                },
            };
            chat.publish_ask(seq, receipt);
        });
        Ok(())
    }

    /// 只解析目标、不碰网络。顺序：壳给的房间 → 本人 Bot 的主房间 → 有权限的默认 Bot。
    /// 有私人 Bot 但还没建好是 `BotNotReady`，不静默换成默认 Bot。
    fn resolve_ask(
        &self,
        request: &AskRequest,
        default_bot: &DefaultBot,
    ) -> Result<AskAttempt, AskProblem> {
        let view = self.view.borrow();
        let own = own_bot(&view);
        match request.room_id.as_deref() {
            Some(raw) => explicit_attempt(&view, raw, own),
            None => own_or_default_attempt(&view, own, default_bot),
        }
    }

    /// 加入 `room` 或新建私聊，`send` 时再发一句话。失败带回试过的房间：回执要说明是哪个房间被拒。
    async fn deliver_ask(
        &self,
        attempt: &AskAttempt,
        body: &str,
        send: bool,
    ) -> Result<OwnedRoomId, (Option<OwnedRoomId>, Error)> {
        let session = self
            .current()
            .map_err(|error| (None, Error::NotReady(error.to_string())))?;
        let room = match (&attempt.room, &attempt.bot) {
            (Some(room), _) => {
                if attempt.join {
                    session
                        .join_room(room)
                        .await
                        .map_err(|error| (Some(room.clone()), error))?;
                }
                room.clone()
            }
            (None, Some(bot)) => session
                .create_direct_chat(bot, false)
                .await
                .map_err(|error| (None, error))?,
            (None, None) => {
                return Err((
                    None,
                    Error::NotReady(l10n!(CATALOG, language(), "chat-ask-target-missing")),
                ));
            }
        };
        if send {
            session
                .send_text_to(&room, body)
                .await
                .map_err(|error| (Some(room.clone()), error))?;
            onboarding::record_sent(&self.sent, room.to_string());
        }
        Ok(room)
    }

    fn next_ask_seq(&self) -> u64 {
        self.ask_seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// 只写还是最新那次动作的回执：慢的旧动作不覆盖新动作。
    fn publish_ask(&self, seq: u64, receipt: AskReceipt) {
        if self.ask_seq.load(Ordering::SeqCst) != seq {
            return;
        }
        self.view.send_modify(|view| view.ask = Some(receipt));
    }

    /// 作废当前回执：用户关掉问题、或者开始新的一次动作时，旧动作迟到的结果不再写回。
    pub(crate) fn invalidate_ask(&self) {
        self.ask_seq.fetch_add(1, Ordering::SeqCst);
        self.view.send_modify(|view| view.ask = None);
    }
}

/// 本人 Bot 的静态事实：用户 ID、主房间、主房间记录的问题。
type OwnBot<'a> = (&'a str, Option<&'a str>, Option<PrivateBotRoomProblem>);

fn own_bot(view: &wonton_protocol::ChatViewModel) -> Option<OwnBot<'_>> {
    match &view.own_private_bot {
        Some(OwnPrivateBotView::Ready {
            user_id,
            home_room,
            room_problem,
            ..
        }) => Some((user_id.as_str(), home_room.as_deref(), *room_problem)),
        _ => None,
    }
}

/// 壳给了房间：是可用 Bot 的私聊或本人 Bot 的主房间就加入它，否则说清不是这个房间。
/// 壳可能给不带服务器的本地房间 ID，按本地部分认；发出去时再补全成完整 ID。
fn explicit_attempt(
    view: &wonton_protocol::ChatViewModel,
    raw: &str,
    own: Option<OwnBot<'_>>,
) -> Result<AskAttempt, AskProblem> {
    let room = full_room_id(raw, view).ok_or_else(|| ask_problem(AskFailure::NotInRoom, None))?;
    if let Some(row) = view.rooms.iter().find(|row| same_room(&row.room_id, raw)) {
        if row.is_read_only {
            return Err(ask_problem(AskFailure::ReadOnlyRoom, None));
        }
        let is_bot = row.bot_chat.is_some()
            || row.direct_user_id.as_deref().is_some_and(|user| {
                Some(user) == own.map(|own| own.0)
                    || view.bots.iter().any(|bot| bot.user_id == user)
            });
        let is_own_room = own.is_some_and(|own| own.1.is_some_and(|home| same_room(home, raw)));
        if !is_bot && !is_own_room {
            return Err(ask_problem(AskFailure::NotInRoom, None));
        }
        return Ok(AskAttempt {
            bot: row
                .direct_user_id
                .as_deref()
                .and_then(|user| user.parse().ok()),
            room: Some(room),
            join: row.membership != RoomMembership::Joined,
        });
    }
    // 这个设备还没同步到这个房间：是本人 Bot 的主房间就等它，不是也先加入再发，失败了按原因报。
    Ok(AskAttempt {
        bot: own.and_then(|own| own.0.parse().ok()),
        room: Some(room),
        join: true,
    })
}

/// 房间 ID 的本地部分相同就当成同一间：Tuwunel 有时给不带服务器的 ID。
fn same_room(left: &str, right: &str) -> bool {
    left == right || left.split(':').next() == right.split(':').next()
}

/// 补齐房间里缺的服务器：`!local` 用当前账号的服务器拼成 `!local:server`。
fn full_room_id(raw: &str, view: &wonton_protocol::ChatViewModel) -> Option<OwnedRoomId> {
    if let Ok(room) = raw.parse() {
        return Some(room);
    }
    let server = match &view.account {
        wonton_protocol::ChatAccount::SignedIn { user_id, .. } => user_id.split_once(':')?.1,
        _ => return None,
    };
    format!("{raw}:{server}").parse().ok()
}

/// 没有壳给的房间：本人 Bot 就绪就用它的主房间，否则按默认 Bot 和读取失败的原因分开处理。
fn own_or_default_attempt(
    view: &wonton_protocol::ChatViewModel,
    own: Option<OwnBot<'_>>,
    default_bot: &DefaultBot,
) -> Result<AskAttempt, AskProblem> {
    if let Some((user_id, home_room, room_problem)) = own {
        let bot = user_id
            .parse::<OwnedUserId>()
            .map_err(|_| ask_problem(AskFailure::BotProblem, None))?;
        match room_problem {
            Some(PrivateBotRoomProblem::NotJoined) => {
                return Err(ask_problem(AskFailure::BotNotReady, None));
            }
            Some(PrivateBotRoomProblem::Invalid | PrivateBotRoomProblem::Other) => {
                return Err(ask_problem(AskFailure::BotProblem, None));
            }
            _ => {}
        }
        if let Some(home_room) = home_room {
            let room = full_room_id(home_room, view)
                .ok_or_else(|| ask_problem(AskFailure::BotProblem, None))?;
            let joined = view.rooms.iter().any(|row| {
                same_room(&row.room_id, home_room) && row.membership == RoomMembership::Joined
            });
            return Ok(AskAttempt {
                bot: Some(bot),
                room: Some(room),
                join: !joined,
            });
        }
        return Ok(AskAttempt {
            bot: Some(bot),
            room: None,
            join: false,
        });
    }
    match &view.own_private_bot {
        // 有私人 Bot、正在建：不静默换成默认 Bot。
        Some(OwnPrivateBotView::Absent) => Err(ask_problem(AskFailure::NoBot, None)),
        // 主人、角色里没有私人 Bot 的人、或者旧核心不给这个键：用默认 Bot。
        Some(OwnPrivateBotView::NotApplicable | OwnPrivateBotView::Other) | None => {
            match default_bot {
                DefaultBot::Visible(bot) => default_attempt(view, bot.clone()),
                DefaultBot::Absent => Err(ask_problem(AskFailure::NoBot, None)),
                DefaultBot::Unknown => match &view.own_private_bot_problem {
                    Some(problem) => Err(ask_problem(own_bot_failure(problem.category), None)),
                    None => Err(ask_problem(AskFailure::BotNotReady, None)),
                },
            }
        }
        Some(OwnPrivateBotView::Ready { .. }) => Err(ask_problem(AskFailure::BotNotReady, None)),
    }
}

/// 默认 Bot：已经有和它的私聊就加入或直接用，没有就新建。
fn default_attempt(
    view: &wonton_protocol::ChatViewModel,
    bot: OwnedUserId,
) -> Result<AskAttempt, AskProblem> {
    let rows: Vec<_> = view
        .rooms
        .iter()
        .filter(|row| !row.is_read_only && row.direct_user_id.as_deref() == Some(bot.as_str()))
        .collect();
    let row = rows
        .iter()
        .find(|row| row.membership == RoomMembership::Joined)
        .or_else(|| {
            rows.iter()
                .find(|row| row.membership == RoomMembership::Invited)
        })
        .or_else(|| rows.first());
    let Some(row) = row else {
        return Ok(AskAttempt {
            bot: Some(bot),
            room: None,
            join: false,
        });
    };
    Ok(AskAttempt {
        bot: Some(bot),
        room: Some(
            full_room_id(&row.room_id, view)
                .ok_or_else(|| ask_problem(AskFailure::NotInRoom, None))?,
        ),
        join: row.membership != RoomMembership::Joined,
    })
}

/// Matrix 错误按原因分类；网关的 `ContactRequired` 从 `errcode` 认出来，其余的 403 是权限。
fn ask_failure(error: &Error) -> AskFailure {
    if let Some(refusal) = error.gateway_refusal() {
        return match refusal {
            GatewayRefusal::ContactRequired => AskFailure::ContactRequired,
            _ => AskFailure::Forbidden,
        };
    }
    match error {
        Error::Transport(_) => AskFailure::TransportUnavailable,
        Error::Unauthorized { .. } | Error::Forbidden { .. } => AskFailure::Forbidden,
        Error::NotReady(_) | Error::NotInvited(_) => AskFailure::NotReady,
        Error::Upstream(_) | Error::CommentNotSent(_) => AskFailure::Upstream,
        _ => AskFailure::Other,
    }
}

/// 读取本人私人 Bot 失败的原因；找不到 Bot 本身和读不出来分开。
fn own_bot_failure(category: OwnBotFailure) -> AskFailure {
    match category {
        OwnBotFailure::TransportUnavailable => AskFailure::TransportUnavailable,
        OwnBotFailure::Unauthorized | OwnBotFailure::PermissionDenied => AskFailure::Forbidden,
        OwnBotFailure::NotReady | OwnBotFailure::CapabilityUnknown => AskFailure::BotNotReady,
        OwnBotFailure::Upstream => AskFailure::Upstream,
        OwnBotFailure::RecordInconsistent
        | OwnBotFailure::StorageUnavailable
        | OwnBotFailure::StorageCorrupt
        | OwnBotFailure::Protocol
        | OwnBotFailure::Other => AskFailure::BotProblem,
    }
}

/// 失败原因配一句按类别写的中文/日文/英文详情；有原始错误时另起一行，给默认收起的“详情”。
fn ask_problem(reason: AskFailure, error: Option<&Error>) -> AskProblem {
    let key = match reason {
        AskFailure::NoBot => "chat-ask-failed-no-bot",
        AskFailure::BotNotReady => "chat-ask-failed-bot-not-ready",
        AskFailure::BotProblem => "chat-ask-failed-bot-problem",
        AskFailure::NotInRoom => "chat-ask-failed-not-in-room",
        AskFailure::ReadOnlyRoom => "chat-ask-failed-read-only-room",
        AskFailure::ContactRequired => "chat-ask-failed-contact-required",
        AskFailure::TransportUnavailable => "chat-ask-failed-transport",
        AskFailure::Forbidden => "chat-ask-failed-forbidden",
        AskFailure::NotReady => "chat-ask-failed-not-ready",
        AskFailure::Upstream => "chat-ask-failed-upstream",
        AskFailure::Other => "chat-ask-failed-other",
    };
    let mut detail = l10n!(CATALOG, language(), key);
    if let Some(error) = error {
        detail.push('\n');
        detail.push_str(&error.to_string());
    }
    AskProblem { reason, detail }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contact_refusal_keeps_its_own_reason() {
        let contact = Error::Forbidden {
            errcode: "APP.WONTON.CONTACT_REQUIRED".to_owned(),
            message: "还不是联系人".to_owned(),
        };
        assert_eq!(ask_failure(&contact), AskFailure::ContactRequired);
        let forbidden = Error::Forbidden {
            errcode: "M_FORBIDDEN".to_owned(),
            message: "没权限".to_owned(),
        };
        assert_eq!(ask_failure(&forbidden), AskFailure::Forbidden);
    }
}
