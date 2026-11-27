/// `GET`：[`Me`](super::Me)。`PUT`：请求体 [`OwnerProfileUpdate`](crate::OwnerProfileUpdate)，改自己的资料，规则同 ADR-0030，返回 [`Me`](super::Me)。
pub const ME_PATH: &str = "/_wonton/api/me";

/// `GET`：[`OwnPrivateBotResponse`](super::OwnPrivateBotResponse)，只读当前人的身份来源。
pub const MY_PRIVATE_BOT_PATH: &str = "/_wonton/api/me/private-bot";

/// `GET`：[`MemberList`](super::MemberList)。`POST`：请求体 [`MemberInvite`](super::MemberInvite)，返回记成“已邀请”的 [`Member`](super::Member)。要管理成员权限。
pub const MEMBERS_PATH: &str = "/_wonton/api/members";

/// `POST`：出一个归这个家人的配对码（[`MemberPairingCode`](super::MemberPairingCode)），之前出的作废。`DELETE`：作废，不出新的。
pub fn member_pairing_code_path(username: &str) -> String {
    format!("{MEMBERS_PATH}/{username}/pairing-code")
}

/// `POST`：请求体 [`MemberRemoval`](super::MemberRemoval)，返回记成“已移除”的 [`Member`](super::Member)。
pub fn member_removal_path(username: &str) -> String {
    format!("{MEMBERS_PATH}/{username}/remove")
}

/// 配对码的有效期。比主人加设备的码（10 分钟）长：要经分享发给不在身边的家人。
pub const MEMBER_PAIRING_CODE_TTL_SECS: u64 = 24 * 60 * 60;

/// `GET`：[`SharedBotsView`](super::SharedBotsView)，按当前请求人列出装着的 Bot 和能不能聊；不受 `open_apps` 过滤，不能聊的也带原因。
/// `POST`：请求体 [`NewSharedBot`](super::NewSharedBot)，返回 [`BotRef`](super::BotRef)。建共享 Bot 只有主人能用。
pub const SHARED_BOTS_PATH: &str = "/_wonton/api/bots";

/// 家庭群的状态事件类型，状态键为空。核心建群时写，App 凭它给这个房间加家庭的标记。
pub const FAMILY_ROOM_EVENT: &str = "app.wonton.family";

/// `POST`：按成员和角色记录重同步家庭群，成功回 204；失败回 503，记录已保存的操作不回滚。要管理成员权限。
pub const FAMILY_ROOM_SYNC_PATH: &str = "/_wonton/api/members/family-room/sync";

/// `POST`：按已保存的成员记录补齐家庭群、私人 Bot、私人空间和移除收尾；成功 204，失败 503。要管理成员权限。
pub const MEMBER_SYNC_PATH: &str = "/_wonton/api/members/sync";

/// 总线转给插件的 `tools/call` 在 `_meta` 里带这一项：私人 Bot 调用时是归属人的用户名，共享 Bot 不带。
pub const TOOL_CALL_USER_META: &str = "app.wonton/user";
