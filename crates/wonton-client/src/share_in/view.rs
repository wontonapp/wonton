//! 分享页的去向：从此刻的连接和聊天状态算出来，不另存一份。

use wonton_protocol::{
    ChatAccount, ChatSync, ChatViewModel, RoomMembership, RoomRow, ShareInDestinations,
};

/// 能发的聊天按聊天列表的顺序（最近有活动的在前）；只读的通知私聊、还没加入的邀请不算。房间列表还空着时，按聊天的状态
/// 分成“还在加载”“用不了”和真的一个都没有。
pub(super) fn destinations(
    paired: bool,
    chat: &ChatViewModel,
    primary_bot: Option<&str>,
    last_room: Option<&str>,
) -> ShareInDestinations {
    if !paired {
        return ShareInDestinations::NotPaired;
    }
    let rooms: Vec<RoomRow> = chat
        .rooms
        .iter()
        .filter(|room| room.membership == RoomMembership::Joined && !room.is_read_only)
        .cloned()
        .collect();
    if rooms.is_empty() {
        match (&chat.account, &chat.sync) {
            (ChatAccount::Failed { message, .. }, _)
            | (ChatAccount::SignedIn { .. }, ChatSync::Failed { message, .. }) => {
                return ShareInDestinations::Unavailable {
                    message: message.clone(),
                };
            }
            (ChatAccount::SignedIn { .. }, ChatSync::Running) => {}
            _ => return ShareInDestinations::Loading,
        }
    }
    let has = |room_id: &str| rooms.iter().any(|room| room.room_id == room_id);
    let suggested_room = last_room
        .filter(|room_id| has(room_id))
        .map(str::to_owned)
        .or_else(|| {
            let bot = primary_bot?;
            rooms
                .iter()
                .find(|room| room.is_direct && room.direct_user_id.as_deref() == Some(bot))
                .map(|room| room.room_id.clone())
        });
    ShareInDestinations::Ready {
        rooms,
        suggested_room,
    }
}
