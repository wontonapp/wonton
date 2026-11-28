use serde::{Deserialize, Serialize};

use crate::{FriendBlockScope, FriendBlockTarget};

/// 好友和屏蔽操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FriendsAction {
    /// 取当前账号的好友、请求和屏蔽名单；外部 Matrix 账号同样可用。
    RefreshFriends,
    /// 发送带 app.wonton.friend 的标准 Matrix 私聊邀请，user_id 必须为完整 Matrix ID。
    AddFriend {
        user_id: String,
    },
    ApproveFriendRequest {
        room_id: String,
    },
    RejectFriendRequest {
        room_id: String,
    },
    CancelFriendRequest {
        room_id: String,
    },
    /// 同时收回这个人所有插件访问，不退出私聊。
    RemoveFriend {
        user_id: String,
    },
    BlockFriend {
        target: FriendBlockTarget,
        scope: FriendBlockScope,
    },
    UnblockFriend {
        target: FriendBlockTarget,
        scope: FriendBlockScope,
    },
}
