//! 发起私聊时选谁：能直接选的人，和把输入的用户名补成完整的 Matrix ID。

use std::collections::HashSet;

use wonton_matrix::{OwnedUserId, UserId};
use wonton_protocol::{ChatContact, CoreError, InstalledPlugin, RoomMembership, RoomRow};

use super::{ChatLink, invalid};

impl ChatLink {
    pub(crate) fn resolve_user_id(&self, input: &str) -> Result<String, CoreError> {
        let session = self.current()?;
        let user = resolve_user(input, session.user_id())?;
        Ok(user.to_string())
    }

    pub(crate) fn contacts(&self, query: &str, plugins: &[InstalledPlugin]) -> Vec<ChatContact> {
        contacts(query, plugins, &self.view.borrow().rooms)
    }
}

/// 已装插件的 Bot 在前，然后是私聊过的人（按最近活动）。通知房间只读，它的对方不算。`query` 不为空时只留名字或
/// ID 里含它的，不分大小写。
pub(crate) fn contacts(
    query: &str,
    plugins: &[InstalledPlugin],
    rooms: &[RoomRow],
) -> Vec<ChatContact> {
    let bots = plugins.iter().filter_map(|plugin| {
        let user_id = plugin.bot_user_id.clone()?;
        Some(ChatContact {
            user_id,
            name: plugin.title.clone(),
        })
    });
    let peers = rooms
        .iter()
        .filter(|room| room.membership == RoomMembership::Joined && !room.is_read_only)
        .filter_map(|room| {
            let user_id = room.direct_user_id.clone()?;
            Some(ChatContact {
                user_id,
                name: room.name.clone(),
            })
        });
    let query = query.trim().to_lowercase();
    let mut seen = HashSet::new();
    bots.chain(peers)
        .filter(|contact| seen.insert(contact.user_id.clone()))
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
        return Err(invalid("用户名不能为空"));
    }
    let full = if name.contains(':') {
        format!("@{name}")
    } else {
        format!("@{}:{}", name.to_lowercase(), own.server_name())
    };
    full.parse()
        .map_err(|error| invalid(&format!("用户 ID 不合法：{error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{PluginKind, PluginState, PluginTrust};

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

    fn plugin(title: &str, bot: Option<&str>) -> InstalledPlugin {
        InstalledPlugin {
            id: title.to_owned(),
            title: title.to_owned(),
            summary: String::new(),
            version: "1".to_owned(),
            revision: "1".to_owned(),
            kind: PluginKind::Agent,
            icon: None,
            trust: PluginTrust::Official,
            source: None,
            provider: None,
            via_index: None,
            written_by: None,
            declares_tools: false,
            status: PluginState::Installing,
            web_url: None,
            web_display: wonton_protocol::TabDisplay::default(),
            cross_origin_isolated: false,
            tabs: Vec::new(),
            bot_user_id: bot.map(str::to_owned),
            family: false,
            upgrade: None,
            removable: true,
            notices: Vec::new(),
        }
    }

    fn room(name: &str, peer: Option<&str>, read_only: bool) -> RoomRow {
        RoomRow {
            room_id: format!("!{name}:s"),
            name: name.to_owned(),
            is_direct: peer.is_some(),
            direct_user_id: peer.map(str::to_owned),
            is_encrypted: true,
            is_read_only: read_only,
            membership: RoomMembership::Joined,
            preview: None,
            unread_count: 0,
        }
    }

    #[test]
    fn bots_come_first_and_everyone_once() {
        let plugins = [plugin("Hermes", Some("@hermes:s")), plugin("相册", None)];
        let rooms = [
            room("Wonton", Some("@wonton-core:s"), true),
            room("Bob", Some("@bob:s"), false),
            room("赫尔墨斯", Some("@hermes:s"), false),
            room("一个群", None, false),
        ];
        let names = |query| -> Vec<String> {
            contacts(query, &plugins, &rooms)
                .into_iter()
                .map(|contact| contact.name)
                .collect()
        };
        assert_eq!(names(""), ["Hermes", "Bob"]);
        assert_eq!(names("HER"), ["Hermes"]);
        assert_eq!(names("@bob"), ["Bob"]);
        assert!(names("没有这个人").is_empty());
    }
}
