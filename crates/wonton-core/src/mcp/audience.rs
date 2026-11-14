//! 确认卡片发给谁、谁发起、显示在哪个会话（ADR-0016 第 2 步，ADR-0045 的“卡片发给归属人”）。
//!
//! 私人 Bot 的普通卡片发给归属人，共享 Bot 和没有 Bot 的插件发给主人。装卸按归属人的角色：能装时问本人，
//! 要管理员批时问创建卡片时的全部管理员。来源会话只认发起的 Bot 和收件人都已加入的房间。

use hyper::HeaderMap;
use serde::Serialize;
use serde_json::Value;
use wonton_protocol::{
    BotKeeper, BotRef, ConfirmationOrigin, ConfirmationRequester, Gate, MemberStatus,
    ORIGIN_ROOM_HEADER, ORIGIN_THREAD_HEADER,
};

use super::confirm::{Audience, Refusal, Ticket};
use super::{Bus, Caller, Draft};
use crate::matrix::BotAccount;
use crate::plugin::languages::Languages;

/// Matrix 的房间 ID、事件 ID 最长 255 字节（Matrix 规范的标识符上限），再长的不是合法标识。
const MAX_ID: usize = 255;

/// 卡片问的是哪类事，决定发给谁（ADR-0045 的权限表）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Subject {
    /// 调插件的工具、Agent 自己的工具：发给 Bot 的归属人。
    Tool,
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
    /// 卡片发给谁：家人的用户名；`None` 是主人。
    pub(super) fn recipient(&self, caller: &Caller, subject: Subject) -> Option<String> {
        let BotKeeper::User { username } = &caller.bot.as_ref()?.keeper else {
            return None;
        };
        if username == self.plugins.owner_name() {
            return None;
        }
        match subject {
            Subject::Tool => Some(username.clone()),
            Subject::PluginAdmin => self.may_install(username).then(|| username.clone()),
        }
    }

    fn may_install(&self, username: &str) -> bool {
        matches!(self.install_gate_for(username), Gate::Allow)
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

    fn plugin_admins(&self) -> Vec<Option<String>> {
        let mut admins = vec![None];
        if let Some(people) = self.people.get() {
            admins.extend(
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
                    .map(|member| Some(member.username)),
            );
        }
        admins
    }

    /// Requests an approval from the administrators present when the card is created.
    pub async fn ask_plugin_admin(
        &self,
        requester: String,
        draft: Draft,
    ) -> Result<Ticket, Refusal> {
        let caller = crate::plugin::PluginId::parse("app.wonton.core")
            .expect("core plugin identifier is valid");
        let mut draft = draft;
        draft.lines.insert(
            0,
            wonton_protocol::CardLine {
                text: requester,
                emphasis: false,
            },
        );
        self.confirmations
            .ask_admin_as(&caller, draft, Audience::default(), self.plugin_admins())
            .await
    }

    pub(super) async fn ask_audience(
        &self,
        caller: &Caller,
        draft: Draft,
        audience: Audience,
        admins: Option<Vec<Option<String>>>,
    ) -> Result<Ticket, Refusal> {
        match admins {
            Some(admins) => {
                self.confirmations
                    .ask_admin_as(&caller.id, draft, audience, admins)
                    .await
            }
            None => self.confirmations.ask_as(&caller.id, draft, audience).await,
        }
    }

    pub(super) fn admin_audience(
        &self,
        caller: &Caller,
        subject: Subject,
    ) -> Option<Vec<Option<String>>> {
        (subject == Subject::PluginAdmin && self.plugin_install_gate(caller) == Gate::AskAdmin)
            .then(|| self.plugin_admins())
    }

    /// 卡片的文字用收件人的语言（ADR-0043）。
    pub(super) fn card_languages(&self, caller: &Caller, subject: Subject) -> Languages {
        self.languages_for(self.recipient(caller, subject).as_deref())
    }

    /// 发卡片要的全部：收件人、发起者和核对过的来源会话。卡片不进任何房间，推送发到收件人的设备（ADR-0016）。
    pub(super) async fn audience(&self, caller: &Caller, subject: Subject) -> Audience {
        let recipient = self.recipient(caller, subject);
        let bot = caller.bot.as_ref().and_then(|bot| {
            let account = self.plugins.bot_account(&caller.id, &bot.bot)?;
            Some((bot, account))
        });
        let (requester, origin) = match bot {
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
                (requester, origin)
            }
            None => (
                ConfirmationRequester::Plugin {
                    plugin_id: caller.id.to_string(),
                },
                None,
            ),
        };
        Audience {
            recipient,
            requester: Some(requester),
            origin,
        }
    }

    /// 请收件人确认（[`Self::audience`]），等到回答或者过期。
    pub(super) async fn ask_for(
        &self,
        caller: &Caller,
        subject: Subject,
        draft: Draft,
    ) -> Result<Ticket, Refusal> {
        let audience = self.audience(caller, subject).await;
        self.ask_audience(
            caller,
            draft,
            audience,
            self.admin_audience(caller, subject),
        )
        .await
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
