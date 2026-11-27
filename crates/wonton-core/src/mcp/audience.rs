//! 确认卡片发给谁、谁发起、显示在哪个会话（ADR-0016 第 2 步，ADR-0045 的“卡片发给归属人”）。
//!
//! 私人 Bot 的普通卡片发给归属人，共享 Bot 和没有 Bot 的插件发给主人。装卸按归属人的角色：能装时问本人，
//! 要管理员批时问创建卡片时的全部管理员。来源会话只认发起的 Bot 和收件人都已加入的房间。

use hyper::HeaderMap;
use serde::Serialize;
use serde_json::Value;
use wonton_protocol::{
    AiLevel, BotKeeper, BotRef, ConfirmationOrigin, ConfirmationRequester, Gate, MemberStatus,
    ORIGIN_ROOM_HEADER, ORIGIN_THREAD_HEADER,
};

use super::confirm::{Audience, Recipients, Refusal, Ticket};
use super::{Bus, Caller, Draft, TurnSender};
use crate::ai_level::{lower, strict_ai};
use crate::matrix::BotAccount;
use crate::plugin::languages::Languages;

/// Matrix 的房间 ID、事件 ID 最长 255 字节（Matrix 规范的标识符上限），再长的不是合法标识。
const MAX_ID: usize = 255;

/// 卡片问的是哪类事，决定发给谁（ADR-0045 的权限表）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Subject {
    /// 调插件的工具、Agent 自己的工具：发给 Bot 的归属人。
    Tool,
    /// 会删、会覆盖的工具调用，在“删除要管理员批”的封顶下：发给所有管理员（ADR-0088 第 5 节）。
    ToolDelete,
    /// 装卸、升级插件，开关插件的权限：按归属人的 install_apps 决定。
    PluginAdmin,
}

/// 这次调用报的来源会话：`_meta` 的 `app.wonton/origin` 为准，没有或者写错了再看请求头。没报、报的不像 Matrix 标识时是
/// `None`。
pub(super) fn reported_origin(
    meta: Option<&Value>,
    headers: Option<&HeaderMap>,
) -> Option<ConfirmationOrigin> {
    let from_meta = meta
        .and_then(|value| serde_json::from_value::<ConfirmationOrigin>(value.clone()).ok())
        .and_then(checked);
    from_meta.or_else(|| {
        let headers = headers?;
        let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        checked(ConfirmationOrigin {
            room_id: text(ORIGIN_ROOM_HEADER)?.to_owned(),
            thread_id: text(ORIGIN_THREAD_HEADER).map(str::to_owned),
        })
    })
}

/// 房间 ID 以 `!` 开头；话题的根事件以 `$` 开头，不像的话题丢掉、房间照用。
pub(super) fn checked(origin: ConfirmationOrigin) -> Option<ConfirmationOrigin> {
    let like = |id: &str, sigil: char| id.len() > 1 && id.len() <= MAX_ID && id.starts_with(sigil);
    if !like(&origin.room_id, '!') {
        return None;
    }
    Some(ConfirmationOrigin {
        thread_id: origin.thread_id.filter(|thread| like(thread, '$')),
        room_id: origin.room_id,
    })
}

/// 报的来源没用上时审计里的一行（ADR-0016：审计里记下报的和用的）。
#[derive(Serialize)]
struct OriginRecord<'a> {
    caller: &'a str,
    event: &'static str,
    reported: &'a ConfirmationOrigin,
    used: Option<&'a ConfirmationOrigin>,
    reason: &'a str,
}

impl Bus {
    /// 卡片普通收件人的用户名；`None` 是主人。管理员卡片另在 [`Self::audience`] 里冻结完整名单。
    pub(crate) fn recipient(&self, caller: &Caller, subject: Subject) -> Option<String> {
        let BotKeeper::User { username } = &caller.bot.as_ref()?.keeper else {
            return None;
        };
        if username == self.plugins.owner_name() {
            return None;
        }
        match subject {
            Subject::Tool | Subject::ToolDelete => Some(username.clone()),
            Subject::PluginAdmin => self.may_install(username).then(|| username.clone()),
        }
    }

    fn may_install(&self, username: &str) -> bool {
        matches!(self.install_gate_for(username), Gate::Allow)
    }

    /// 一个人的 AI 动手级别（ADR-0088 第 5 节“谁让 Bot 做的事，按谁的角色封顶”）：主人不受角色约束，
    /// 其余按角色；人查不到、角色查不到、取值不认识时按最严并记日志，不按最松的放行。
    pub(crate) fn person_ai(&self, username: &str) -> AiLevel {
        crate::ai_level::person_ai(
            self.plugins.owner_name(),
            self.people
                .get()
                .map(|people| (&*people.members, &*people.roles)),
            username,
        )
    }

    /// 这次调用的封顶（ADR-0088 第 5 节、ADR-0083 第 5 节）：报了说话人按这个人算；没报时按进行中的轮次里
    /// 级别最低的人算；两样都没有（[`Self::speaker_missing`]）按只读。没有 Bot 的工具插件按主人的自动化算。
    /// 说话人本身是本机 Bot 的账号时按它那轮所在链的发起人算（ADR-0061 第 10 节）。
    ///
    /// 说不出替谁办事时不按 Bot 归属人或共享 Bot 的能用人算：插件取不到身份不带头、开链失败没报上轮次，
    /// 这一轮就会拿到归属人（主人的 Bot 就是主人）的权限，等于身份升级。按只读失败即停，Agent 照实转告。
    pub(crate) fn ai_cap(&self, caller: &Caller) -> AiLevel {
        if let Some(level) = caller.ai_level {
            return strict_ai(level);
        }
        let Some(bot) = &caller.bot else {
            return AiLevel::SelfConfirms;
        };
        if let Some(sender) = &caller.sender {
            return match sender {
                TurnSender::Local(user_id) => {
                    if self.plugins.bot_by_user(user_id).is_some() {
                        self.bot_speaker_ai(caller, user_id)
                    } else {
                        localpart(user_id)
                            .map_or(AiLevel::ReadOnly, |username| self.person_ai(username))
                    }
                }
                TurnSender::Unknown => AiLevel::ReadOnly,
            };
        }
        match &bot.keeper {
            BotKeeper::User { .. } | BotKeeper::Household => {
                let turns = self.live_turn_ais(caller);
                turns.into_iter().reduce(lower).unwrap_or_else(|| {
                    tracing::warn!(
                        plugin = caller.id.as_str(),
                        "这次调用没说替谁办事（没带说话人请求头，也没有进行中的轮次），AI 动手按只读封顶"
                    );
                    AiLevel::ReadOnly
                })
            }
            BotKeeper::Other => {
                tracing::warn!(
                    plugin = caller.id.as_str(),
                    "认不出 Bot 的归属人，AI 动手按只读封顶"
                );
                AiLevel::ReadOnly
            }
        }
    }

    /// 说话的人本身是本机 Bot 的账号（Bot 之间传话）：按它那一轮所在链的发起人算。先看这次调用认下的链，
    /// 认不下时看它其余的进行中轮次，几轮分不清时取最严。找不到链、链代表认不出时按只读。
    fn bot_speaker_ai(&self, caller: &Caller, sender: &str) -> AiLevel {
        let Some(chains) = self.agent_chains.get() else {
            return AiLevel::ReadOnly;
        };
        let ledger = chains.ledger();
        let Some(bot) = caller.bot.as_ref().map(|identity| BotRef {
            plugin_id: caller.id.to_string(),
            bot: identity.bot.clone(),
        }) else {
            return AiLevel::ReadOnly;
        };
        let turns: Vec<_> = ledger
            .live_turns(&bot)
            .into_iter()
            .filter(|turn| turn.sender == sender)
            .collect();
        let picked: Vec<_> = match caller.chain.as_deref() {
            Some(chain) if turns.iter().any(|turn| turn.chain == chain) => {
                turns.iter().filter(|turn| turn.chain == chain).collect()
            }
            _ => turns.iter().collect(),
        };
        picked
            .into_iter()
            .filter_map(|turn| ledger.chain_user(&turn.chain))
            .map(|user| {
                localpart(&user).map_or(AiLevel::ReadOnly, |username| self.person_ai(username))
            })
            .reduce(lower)
            .unwrap_or(AiLevel::ReadOnly)
    }

    /// 进行中的轮次各自代表的级别。说话的人认不出时按最严，不用链代表的人替代（ChainSessionInfo 的 `sender`
    /// 是这条轮次的说话人；认不清时不能按更松的链身份放行）。
    fn live_turn_ais(&self, caller: &Caller) -> Vec<AiLevel> {
        let Some(chains) = self.agent_chains.get() else {
            return Vec::new();
        };
        let ledger = chains.ledger();
        let Some(bot) = caller.bot.as_ref().map(|identity| BotRef {
            plugin_id: caller.id.to_string(),
            bot: identity.bot.clone(),
        }) else {
            return Vec::new();
        };
        ledger
            .live_turns(&bot)
            .into_iter()
            .map(|turn| {
                let Some(username) = localpart(&turn.sender) else {
                    tracing::warn!(
                        plugin = caller.id.as_str(),
                        sender = turn.sender.as_str(),
                        "认不出这轮说话的人，AI 动手按只读封顶"
                    );
                    return AiLevel::ReadOnly;
                };
                self.person_ai_option(username).unwrap_or_else(|| {
                    tracing::warn!(
                        plugin = caller.id.as_str(),
                        sender = turn.sender.as_str(),
                        "认不出这轮说话的人，AI 动手按只读封顶"
                    );
                    AiLevel::ReadOnly
                })
            })
            .collect()
    }

    /// 有 Bot 记录的调用方说不出这次替谁办事：没报说话人，这个 Bot 也没有进行中的轮次。
    pub(super) fn speaker_missing(&self, caller: &Caller) -> bool {
        caller.bot.is_some() && caller.sender.is_none() && self.live_turn_ais(caller).is_empty()
    }

    /// 封顶只读时的拒绝（权限缺失）。说不出替谁办事的写明这一点，Agent 能照实转告，不当成角色不够。
    pub(super) fn read_only_refusal(&self, caller: &Caller, refused: &str) -> super::ToolFailure {
        let why = if self.speaker_missing(caller) {
            "这次调用没说替谁办事（没带说话人请求头，也没有进行中的轮次），AI 动手按只读"
        } else {
            "这次的 AI 动手级别是只读"
        };
        super::ToolFailure::new("permission_denied", format!("{why}，{refused}，没有执行"))
    }

    fn person_ai_option(&self, username: &str) -> Option<AiLevel> {
        let known = username == self.plugins.owner_name()
            || self
                .people
                .get()
                .and_then(|people| people.members.get(username))
                .is_some_and(|member| {
                    matches!(member.status, MemberStatus::Invited | MemberStatus::Joined)
                });
        known.then(|| self.person_ai(username))
    }

    fn install_gate_for(&self, username: &str) -> Gate {
        if username == self.plugins.owner_name() {
            return Gate::Allow;
        }
        self.people
            .get()
            .and_then(|people| {
                let member = people.members.get(username)?;
                if member.status != MemberStatus::Joined {
                    return None;
                }
                people
                    .roles
                    .get(&member.role_id)
                    .map(|role| role.permissions.install_apps)
            })
            .unwrap_or(Gate::Deny)
    }

    pub(crate) fn plugin_install_gate(&self, caller: &Caller) -> Gate {
        match caller.bot.as_ref().map(|bot| &bot.keeper) {
            Some(BotKeeper::User { username }) => self.install_gate_for(username),
            None | Some(BotKeeper::Household) => Gate::Allow,
            Some(BotKeeper::Other) => Gate::Deny,
        }
    }

    pub(super) fn require_plugin_install(&self, caller: &Caller) -> Result<(), super::ToolFailure> {
        match self.plugin_install_gate(caller) {
            Gate::Allow | Gate::AskAdmin => Ok(()),
            Gate::Deny | Gate::Other => Err(super::ToolFailure::new(
                "permission_denied",
                "归属人的角色不允许装、卸载或更改应用",
            )),
        }
    }

    pub(crate) fn plugin_admins(&self) -> Vec<String> {
        self.people
            .get()
            .map(|people| {
                people
                    .members
                    .list()
                    .into_iter()
                    .filter(|member| {
                        member.username != self.plugins.owner_name()
                            && member.status == MemberStatus::Joined
                            && people
                                .roles
                                .get(&member.role_id)
                                .is_some_and(|role| role.permissions.manage_members)
                    })
                    .map(|member| member.username)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 把 `acted_by` 的原串换成卡片上的“替谁”：家人经成员记录换成显示名，换成“替某某”；
    /// 主人令牌自己办的不写行。换不出来时保留原串，至少不丢是谁。
    fn with_requester_line(
        &self,
        mut draft: Draft,
        raw: &str,
    ) -> (Draft, Option<wonton_protocol::OnBehalfView>) {
        if raw == "owner_token" {
            return (draft, None);
        }
        let username = raw.split_once("/device:").map(|(username, _)| username);
        let on_behalf = username.and_then(|username| {
            let people = self.people.get()?;
            let member = people
                .members
                .list()
                .into_iter()
                .find(|member| member.username == username)?;
            Some(wonton_protocol::OnBehalfView {
                user_id: member.user_id,
                display_name: member.display_name,
            })
        });
        if on_behalf.is_none() {
            draft.lines.insert(
                0,
                wonton_protocol::CardLine {
                    text: raw.to_owned(),
                    emphasis: false,
                },
            );
        }
        (draft, on_behalf)
    }

    /// Requests an approval from the administrators present when the card is created.
    pub async fn ask_plugin_admin(
        &self,
        requester: String,
        draft: Draft,
    ) -> Result<Ticket, Refusal> {
        let caller = crate::plugin::PluginId::parse("app.wonton.core")
            .expect("core plugin identifier is valid");
        let (draft, on_behalf) = self.with_requester_line(draft, &requester);
        let audience = Audience {
            recipients: Recipients::Admins(self.plugin_admins()),
            on_behalf,
            ..Audience::default()
        };
        self.confirmations.ask_as(&caller, draft, audience).await
    }

    pub(crate) async fn ask_plugin_admin_twice(
        &self,
        requester: String,
        first: Draft,
        second: Draft,
    ) -> Result<(Ticket, Ticket), Refusal> {
        let caller = crate::plugin::PluginId::parse("app.wonton.core")
            .expect("core plugin identifier is valid");
        let (mut first, on_behalf) = self.with_requester_line(first, &requester);
        let audience = Audience {
            recipients: Recipients::Admins(self.plugin_admins()),
            on_behalf,
            ..Audience::default()
        };
        let first = self
            .confirmations
            .ask_as(&caller, first, audience.clone())
            .await?;
        match self.confirmations.ask_as(&caller, second, audience).await {
            Ok(second) => Ok((first, second)),
            Err(refusal) => {
                self.confirmations
                    .finish(
                        first,
                        wonton_protocol::ConfirmationState::Failed {
                            message: refusal.message(),
                        },
                    )
                    .await;
                Err(refusal)
            }
        }
    }

    /// 这个 Bot 替谁办事：有归属家人的 Bot 换成那个人的显示名，主人和全家共享的 Bot 不写。
    fn on_behalf_of(&self, keeper: &BotKeeper) -> Option<wonton_protocol::OnBehalfView> {
        let BotKeeper::User { username } = keeper else {
            return None;
        };
        if username == self.plugins.owner_name() {
            return None;
        }
        let people = self.people.get()?;
        let member = people
            .members
            .list()
            .into_iter()
            .find(|member| &member.username == username)?;
        Some(wonton_protocol::OnBehalfView {
            user_id: member.user_id,
            display_name: member.display_name,
        })
    }

    /// 卡片的文字用谁的语言（ADR-0043）：发给管理员的（会删工具的、安装的）按主人的语言，其余发给谁按谁的。
    pub(super) fn card_languages(&self, caller: &Caller, subject: Subject) -> Languages {
        let recipient = match subject {
            Subject::ToolDelete => None,
            _ => self.recipient(caller, subject),
        };
        self.languages_for(recipient.as_deref())
    }

    /// 发卡片要的全部：收件人、发起者和核对过的来源会话。卡片不进任何 Matrix 房间，推送发到冻结收件人的设备。
    pub(super) async fn audience(&self, caller: &Caller, subject: Subject) -> Audience {
        let recipient = self.recipient(caller, subject);
        let recipients = match subject {
            // 会删、会覆盖的工具调用按封顶发管理员，谁先答算谁的（ADR-0088 第 5 节）。
            Subject::ToolDelete => Recipients::Admins(self.plugin_admins()),
            Subject::PluginAdmin if self.plugin_install_gate(caller) == Gate::AskAdmin => {
                Recipients::Admins(self.plugin_admins())
            }
            _ => Recipients::Keeper(recipient.clone()),
        };
        let bot = caller.bot.as_ref().and_then(|bot| {
            let account = self.plugins.bot_account(&caller.id, &bot.bot)?;
            Some((bot, account))
        });
        let (requester, origin, on_behalf) = match bot {
            Some((identity, account)) => {
                let shares_home = self.home_room_is_theirs(&identity.keeper, recipient.as_deref());
                let origin = self
                    .checked_origin(caller, &account, shares_home, recipient.as_deref())
                    .await;
                let requester = ConfirmationRequester::Bot {
                    bot: BotRef {
                        plugin_id: caller.id.to_string(),
                        bot: identity.bot.clone(),
                    },
                    user_id: account.user_id,
                };
                let on_behalf = self.on_behalf_of(&identity.keeper);
                (requester, origin, on_behalf)
            }
            None => (
                ConfirmationRequester::Plugin {
                    plugin_id: caller.id.to_string(),
                },
                None,
                None,
            ),
        };
        Audience {
            recipients,
            requester: Some(requester),
            on_behalf,
            origin,
            category: None,
        }
    }

    /// 请冻结收件人确认，等到回答或者过期。卡片认到调用头里的链，停链时随停撤回。
    pub(super) async fn ask_for(
        &self,
        caller: &Caller,
        subject: Subject,
        draft: Draft,
    ) -> Result<Ticket, Refusal> {
        let audience = self.audience(caller, subject).await;
        let (card, answered) = self.confirmations.open_as(&caller.id, draft, audience)?;
        // 停链抢在开卡片和绑卡片之间时，停链那一遍已经收不到这张卡片了：绑上之后回查，是停的就当场撤回。
        if super::chain_card::bind_card(self, caller, None, &card.id, "工具卡片").outcome
            == super::chain_card::Bound::Withdrawn
        {
            return Err(Refusal::Cancelled);
        }
        self.confirmations.wait(card, answered).await
    }

    /// Bot 的主房间是它和归属人的私聊（共享 Bot 是和主人的）：收件人是这个人时，主房间就是两边都在的房间。
    fn home_room_is_theirs(&self, keeper: &BotKeeper, recipient: Option<&str>) -> bool {
        let owner = self.plugins.owner_name();
        let recipient = recipient.unwrap_or(owner);
        match keeper {
            BotKeeper::User { username } => username == recipient,
            BotKeeper::Household | BotKeeper::Other => recipient == owner,
        }
    }

    async fn checked_origin(
        &self,
        caller: &Caller,
        account: &BotAccount,
        shares_home: bool,
        recipient: Option<&str>,
    ) -> Option<ConfirmationOrigin> {
        let home = shares_home.then(|| ConfirmationOrigin {
            room_id: account.home_room.clone(),
            thread_id: None,
        });
        // 没报来源：落到主房间，不算没用上，审计不记。
        let Some(reported) = caller.origin.as_ref() else {
            return home;
        };
        let checked = if shares_home && reported.room_id == account.home_room {
            Ok(())
        } else {
            self.both_joined(&account.user_id, recipient, &reported.room_id)
                .await
        };
        match checked {
            Ok(()) => Some(reported.clone()),
            Err(reason) => {
                tracing::warn!(caller = %caller.id, room = %reported.room_id, %reason, "报的来源会话没用上，卡片落到 Bot 的主房间");
                self.audit.record(&OriginRecord {
                    caller: caller.id.as_str(),
                    event: "confirmation_origin_rejected",
                    reported,
                    used: home.as_ref(),
                    reason: &reason,
                });
                home
            }
        }
    }

    /// 以 Bot 的身份查房间成员：Bot 和收件人都已加入才算。核心临时代 Bot 登录一台设备，查完退出（同 Bot 钉页的卡片）。
    async fn both_joined(
        &self,
        bot_user: &str,
        recipient: Option<&str>,
        room: &str,
    ) -> Result<(), String> {
        let cards = self.cards()?;
        let matrix = cards
            .matrix
            .clone()
            .ok_or_else(|| "service_unavailable: the core has no Matrix connection".to_owned())?;
        let localpart = bot_user
            .strip_prefix('@')
            .and_then(|rest| rest.split_once(':'))
            .map(|(localpart, _)| localpart)
            .ok_or_else(|| format!("invalid_input: {bot_user} is not a Matrix user ID"))?;
        let person = matrix.user_id(recipient.unwrap_or(&cards.owner));
        let login = matrix
            .login(localpart, "Wonton 核心（核对来源会话）")
            .await
            .map_err(|e| format!("service_unavailable: {e}"))?;
        let session = matrix.session(&login);
        let members = session.joined_members(room).await;
        if let Err(error) = session.logout().await {
            tracing::warn!(bot = %bot_user, %error, "核对来源会话后没能退出临时设备");
        }
        let members = members.map_err(|e| format!("not_a_member: {e}"))?;
        if members.iter().any(|m| m == bot_user) && members.contains(&person) {
            Ok(())
        } else {
            Err("not_shared: the bot and the recipient are not both in this room".to_owned())
        }
    }
}

/// Matrix 用户 ID（`@localpart:服务器名`）里的用户名；不是这个写法返回 `None`，让查角色时按认不出处理。
pub(super) fn localpart(user_id: &str) -> Option<&str> {
    let rest = user_id.strip_prefix('@')?;
    let (localpart, server) = rest.split_once(':')?;
    (!localpart.is_empty() && !server.is_empty()).then_some(localpart)
}

#[cfg(test)]
mod tests {
    use hyper::header::HeaderValue;
    use serde_json::json;

    use super::*;

    fn headers(room: &str, thread: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(ORIGIN_ROOM_HEADER, HeaderValue::from_str(room).unwrap());
        if let Some(thread) = thread {
            map.insert(ORIGIN_THREAD_HEADER, HeaderValue::from_str(thread).unwrap());
        }
        map
    }

    #[test]
    fn meta_wins_over_headers() {
        let meta = json!({"room_id": "!meta:s", "thread_id": "$t"});
        let origin = reported_origin(Some(&meta), Some(&headers("!header:s", None))).unwrap();
        assert_eq!(origin.room_id, "!meta:s");
        assert_eq!(origin.thread_id.as_deref(), Some("$t"));
    }

    #[test]
    fn headers_are_used_when_meta_is_missing_or_malformed() {
        let from = |meta: Option<Value>| {
            reported_origin(meta.as_ref(), Some(&headers("!h:s", Some("$root"))))
        };
        assert_eq!(from(None).unwrap().room_id, "!h:s");
        assert_eq!(from(Some(json!({"room": "!x:s"}))).unwrap().room_id, "!h:s");
        assert_eq!(from(Some(json!({"room_id": "x"}))).unwrap().room_id, "!h:s");
        assert_eq!(from(None).unwrap().thread_id.as_deref(), Some("$root"));
    }

    #[test]
    fn ids_that_are_not_matrix_ids_are_dropped() {
        assert_eq!(reported_origin(None, Some(&headers("general", None))), None);
        let bad_thread = json!({"room_id": "!r:s", "thread_id": "root"});
        let origin = reported_origin(Some(&bad_thread), None).unwrap();
        assert_eq!((origin.room_id.as_str(), origin.thread_id), ("!r:s", None));
        let long = json!({"room_id": format!("!{}", "a".repeat(MAX_ID))});
        assert_eq!(reported_origin(Some(&long), None), None);
        assert_eq!(reported_origin(None, None), None);
    }
}
