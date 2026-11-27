//! 发起私聊时选谁：能直接选的人，和把输入的用户名补成完整的 Matrix ID。

use std::collections::HashSet;

use wonton_l10n::l10n;
use wonton_matrix::{OwnedUserId, UserId};
use wonton_protocol::{
    BotAccess, BotChatAvailability, BotView, ChatContact, CoreError, OwnPrivateBotView,
    RoomMembership, RoomRow, SharedBot,
};

use super::{ChatLink, invalid};
use crate::l10n::{CATALOG, language};

impl ChatLink {
    pub(crate) fn resolve_user_id(&self, input: &str) -> Result<String, CoreError> {
        let session = self.current()?;
        let user = resolve_user(input, session.user_id())?;
        Ok(user.to_string())
    }

    pub(crate) fn contacts(&self, query: &str) -> Vec<ChatContact> {
        let view = self.view.borrow();
        let query = query.trim().to_lowercase();
        view.shared_contacts
            .iter()
            .filter(|contact| {
                contact.name.to_lowercase().contains(&query) || contact.user_id.contains(&query)
            })
            .cloned()
            .collect()
    }
}

/// 把核心给的可用性、本人 Bot 和私聊过的人合成联系人表：能用的在前，不能用的带原因留在表里，
/// 按名字和用户 ID 去重。本人 Bot 已经在可达清单里时不重复。
#[allow(clippy::too_many_lines)]
pub(crate) fn assemble(
    bots: &[BotView],
    shared: Option<&[SharedBot]>,
    rooms: &[RoomRow],
    own: Option<&OwnPrivateBotView>,
) -> Vec<ChatContact> {
    // 和对方的私聊里对方声明了支持旁聊，发起时就在那个房间里开旁聊（ADR-0093 §4）；否则壳走 StartDirectChat。
    let side_chat_room = |user_id: &str| {
        rooms
            .iter()
            .find(|room| {
                room.side_chats
                    && room.membership == RoomMembership::Joined
                    && room.direct_user_id.as_deref() == Some(user_id)
            })
            .map(|room| room.room_id.clone())
    };
    let name_of = |user_id: &str, fallback: &str| {
        bots.iter()
            .find(|bot| bot.user_id == user_id)
            .map_or_else(|| fallback.to_owned(), |bot| bot.name.clone())
    };
    let mut seen = HashSet::new();
    let mut contacts = Vec::new();
    if let Some(shared) = shared {
        for row in shared {
            // Bot 记录还没建出来时没有用户 ID：留一行让人看见它，只是还不能点。
            let key = row
                .user_id
                .clone()
                .unwrap_or_else(|| format!("{}/{}", row.bot.plugin_id, row.bot.bot));
            if !seen.insert(key) {
                continue;
            }
            let fallback = if row.bot.bot == "default" {
                row.title.clone()
            } else {
                format!("{} · {}", row.title, row.bot.bot)
            };
            let name = row
                .user_id
                .as_deref()
                .map_or_else(|| fallback.clone(), |user_id| name_of(user_id, &fallback));
            contacts.push(ChatContact {
                side_chat_room_id: row.user_id.as_deref().and_then(side_chat_room),
                user_id: row.user_id.clone().unwrap_or_default(),
                name,
                bot: Some(row.bot.clone()),
                access: row.access,
            });
        }
    }

    if let Some(OwnPrivateBotView::Ready { user_id, bot, .. }) = own
        && seen.insert(user_id.clone())
    {
        contacts.push(ChatContact {
            side_chat_room_id: side_chat_room(user_id),
            user_id: user_id.clone(),
            name: name_of(user_id, &l10n!(CATALOG, language(), "chat-my-bot")),
            bot: Some(bot.clone()),
            access: BotAccess::Own,
        });
    }
    for bot in bots {
        if seen.contains(&bot.user_id) {
            continue;
        }
        // 旧核心没有可达清单时沿用旧口径：不能聊的 Bot 不列，未知照列。
        if shared.is_none() && matches!(&bot.availability, BotChatAvailability::Unavailable { .. })
        {
            continue;
        }
        seen.insert(bot.user_id.clone());
        contacts.push(ChatContact {
            side_chat_room_id: side_chat_room(&bot.user_id),
            user_id: bot.user_id.clone(),
            name: bot.name.clone(),
            bot: Some(bot.bot.clone()),
            access: bot.availability.into(),
        });
    }
    for room in rooms {
        if room.membership != RoomMembership::Joined || room.is_read_only {
            continue;
        }
        let Some(user_id) = room.direct_user_id.clone() else {
            continue;
        };
        if !seen.insert(user_id.clone()) {
            continue;
        }
        // 旧核心没有可达清单时，对方是当前不能用的 Bot 就不当普通人再列一次。
        if shared.is_none()
            && bots.iter().any(|bot| {
                bot.user_id == user_id
                    && matches!(&bot.availability, BotChatAvailability::Unavailable { .. })
            })
        {
            continue;
        }
        contacts.push(ChatContact {
            side_chat_room_id: room.side_chats.then(|| room.room_id.clone()),
            user_id,
            name: room.name.clone(),
            bot: None,
            access: BotAccess::Unknown,
        });
    }
    contacts.sort_by_key(|contact| match contact.access {
        BotAccess::Own => 0,
        BotAccess::Shared => 1,
        BotAccess::Contact => 2,
        BotAccess::Unknown => 3,
        BotAccess::Unusable { .. } => 4,
    });
    contacts
}

/// 旧口径的合成：没有可达清单时按可用性和私聊过的人出联系人，只给测试用。
#[cfg(test)]
pub(crate) fn contacts(query: &str, bots: &[BotView], rooms: &[RoomRow]) -> Vec<ChatContact> {
    let query = query.trim().to_lowercase();
    let assembled = assemble(bots, None, rooms, None);
    assembled
        .into_iter()
        .filter(|contact| {
            contact.name.to_lowercase().contains(&query) || contact.user_id.contains(&query)
        })
        .collect()
}

/// 输入的是用户名时补上本账号的服务器；带服务器的按完整 ID 算，开头的 `@` 可以省。Matrix 的用户名都是小写，
/// 用户名按小写算；完整 ID 的大小写不动，别的服务器可能有历史上的大写 ID。
pub(crate) fn resolve_user(input: &str, own: &UserId) -> Result<OwnedUserId, CoreError> {
    let input = input.trim();
    let name = input.strip_prefix('@').unwrap_or(input);
    if name.is_empty() {
        return Err(invalid(&l10n!(
            CATALOG,
            language(),
            "chat-contacts-empty-username"
        )));
    }
    let full = if name.contains(':') {
        format!("@{name}")
    } else {
        format!("@{}:{}", name.to_lowercase(), own.server_name())
    };
    full.parse::<OwnedUserId>().map_err(|error| {
        invalid(&l10n!(
            CATALOG,
            language(),
            "chat-contacts-invalid-user-id",
            reason = error.to_string()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(input: &str) -> Result<String, CoreError> {
        let own: OwnedUserId = "@owner:wonton.localhost".parse().unwrap();
        resolve_user(input, &own).map(|user| user.to_string())
    }

    #[test]
    fn a_username_gets_the_home_server() {
        for input in ["hermes", " Hermes ", "@hermes"] {
            assert_eq!(resolved(input).unwrap(), "@hermes:wonton.localhost");
        }
    }

    #[test]
    fn a_full_id_stays_as_typed() {
        for input in ["@Bob:other.example", "Bob:other.example"] {
            assert_eq!(resolved(input).unwrap(), "@Bob:other.example");
        }
    }

    #[test]
    fn what_cannot_be_an_id_is_refused() {
        for input in ["", "@", "bob:"] {
            assert!(
                matches!(resolved(input), Err(CoreError::InvalidInput { .. })),
                "{input:?}"
            );
        }
    }

    fn bot(title: &str, user: &str, availability: BotChatAvailability) -> BotView {
        BotView {
            user_id: user.to_owned(),
            bot: wonton_protocol::BotRef {
                plugin_id: "app.bot".into(),
                bot: "default".into(),
            },
            availability,
            web: None,
            web_page: None,
            name: title.to_owned(),
            avatar: None,
            state_avatars: Vec::new(),
            activity: None,
            saving: false,
            problem: None,
            chat: Default::default(),
        }
    }

    fn room(name: &str, peer: Option<&str>, read_only: bool) -> RoomRow {
        RoomRow {
            unread_mentions: 0,
            layout: Default::default(),
            call_active: false,
            room_id: format!("!{name}:s"),
            name: name.to_owned(),
            is_direct: peer.is_some(),
            direct_user_id: peer.map(str::to_owned),
            plugin: None,
            bot_chat: None,
            bot: None,
            is_encrypted: true,
            is_read_only: read_only,
            membership: RoomMembership::Joined,
            preview: None,
            side_chats: false,
            unread_count: 0,
        }
    }

    #[test]
    fn available_bots_come_first_and_everyone_once() {
        let bots = [
            bot("Hermes", "@hermes:s", BotChatAvailability::Available),
            bot(
                "（归档的）",
                "@old:s",
                BotChatAvailability::Unavailable {
                    reason: wonton_protocol::BotChatUnavailable::GrantNotActive,
                },
            ),
        ];
        let rooms = [
            room("Wonton", Some("@wonton-core:s"), true),
            room("Bob", Some("@bob:s"), false),
            room("赫尔墨斯", Some("@hermes:s"), false),
            room("一个群", None, false),
        ];
        let names = |query| -> Vec<String> {
            contacts(query, &bots, &rooms)
                .into_iter()
                .map(|c| c.name)
                .collect()
        };
        assert_eq!(names(""), ["Hermes", "Bob"]);
        assert_eq!(names("HER"), ["Hermes"]);
        assert_eq!(names("@bob"), ["Bob"]);
        assert!(names("没有这个人").is_empty());
    }

    #[test]
    fn unavailable_bot_stays_out_and_unknown_stays_in_as_one_contact() {
        let bots = [
            bot(
                "主人的助手",
                "@private:s",
                BotChatAvailability::Unavailable {
                    reason: wonton_protocol::BotChatUnavailable::OwnerPrivate,
                },
            ),
            bot("还没核实", "@unsure:s", BotChatAvailability::Unknown),
        ];
        let rooms = [
            room("主人的助手", Some("@private:s"), false),
            room("还没核实", Some("@unsure:s"), false),
        ];
        let names: Vec<_> = contacts("", &bots, &rooms)
            .into_iter()
            .map(|c| c.name)
            .collect();
        // 未知的照常能发起（断网会进离线队列），房间也不再当人重复列一次。
        assert_eq!(names, ["还没核实"]);
    }

    fn shared(title: &str, user: &str, bot: &str, access: BotAccess) -> SharedBot {
        SharedBot {
            bot: wonton_protocol::BotRef {
                plugin_id: "app.bot".into(),
                bot: bot.into(),
            },
            user_id: Some(user.into()),
            title: title.into(),
            icon: None,
            access,
        }
    }

    #[test]
    fn shared_list_keeps_reasons_and_is_sorted_by_access() {
        let bots = [bot("Hermes", "@hermes:s", BotChatAvailability::Available)];
        let rooms = [room("主人的助手", Some("@private:s"), false)];
        let shared = [
            shared(
                "主人的助手",
                "@private:s",
                "default",
                BotAccess::Unusable {
                    reason: wonton_protocol::BotChatUnavailable::OwnerPrivate,
                },
            ),
            shared("Hermes", "@hermes:s", "default", BotAccess::Shared),
        ];
        let contacts = assemble(&bots, Some(&shared), &rooms, None);
        let names: Vec<_> = contacts
            .iter()
            .map(|contact| contact.name.as_str())
            .collect();
        assert_eq!(names, ["Hermes", "主人的助手"]);
        assert_eq!(contacts[0].access, BotAccess::Shared);
        assert_eq!(
            contacts[1].access,
            BotAccess::Unusable {
                reason: wonton_protocol::BotChatUnavailable::OwnerPrivate
            }
        );
    }

    #[test]
    fn a_missing_bot_record_still_leaves_a_visible_row() {
        let shared = [SharedBot {
            bot: wonton_protocol::BotRef {
                plugin_id: "app.bot".into(),
                bot: "default".into(),
            },
            user_id: None,
            title: "共享助手".into(),
            icon: None,
            access: BotAccess::Unusable {
                reason: wonton_protocol::BotChatUnavailable::BotMissing,
            },
        }];
        let contacts = assemble(&[], Some(&shared), &[], None);
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].user_id.is_empty());
        assert_eq!(contacts[0].name, "共享助手");
        assert_eq!(
            contacts[0].access,
            BotAccess::Unusable {
                reason: wonton_protocol::BotChatUnavailable::BotMissing
            }
        );
    }

    #[test]
    fn own_bot_is_added_once_with_a_name() {
        let bots = [bot("小助手", "@mine:s", BotChatAvailability::Available)];
        let own = OwnPrivateBotView::Ready {
            bot: wonton_protocol::BotRef {
                plugin_id: "app.bot".into(),
                bot: "default".into(),
            },
            user_id: "@mine:s".into(),
            home_room: None,
            room_problem: None,
        };
        let contacts = assemble(&bots, None, &[], Some(&own));
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].name, "小助手");
        assert_eq!(contacts[0].access, BotAccess::Own);
    }
}
