//! 家人和多人（ADR-0045）：每人一个账号，Bot 有归属人，同机的 Bot 经联系人互发消息。
//!
//! - [`ME_PATH`]：这台设备的人是谁、是不是主人、什么角色、能做什么，每个人都能读自己的。
//! - [`MEMBERS_PATH`] 之下：管理成员权限可邀请、管理、移除成员和出码；邀请时指定角色只收主人。
//! - [`SHARED_BOTS_PATH`]：主人建共享 Bot。Bot 的联系人在 [`crate::bot_contacts_path`]，归属人管。
//! - 网关拒绝 Bot 的 Matrix 请求时，在 [`GATEWAY_REFUSAL_HEADER`] 里写 [`GatewayRefusal`]；跳数到顶另用
//!   [`HOP_LIMIT_ERRCODE`] 作 `errcode`。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`。

mod bot;
mod me;
mod member;
mod paths;
mod refusal;
mod shared_bot;
#[cfg(test)]
mod tests;
mod view_model;

pub use bot::{
    BotKeeper, BotRef, OwnPrivateBotResponse, OwnPrivateBotResult, PrivateBotIdentity,
    PrivateBotRoomProblem,
};
pub use me::{Me, MyAvatarProblem, Role};
pub use member::{
    Member, MemberInvite, MemberList, MemberPairingCode, MemberRemoval, MemberStatus,
};
pub use paths::{
    FAMILY_ROOM_EVENT, FAMILY_ROOM_SYNC_PATH, ME_PATH, MEMBER_PAIRING_CODE_TTL_SECS,
    MEMBER_SYNC_PATH, MEMBERS_PATH, MY_PRIVATE_BOT_PATH, SHARED_BOTS_PATH, TOOL_CALL_USER_META,
    member_pairing_code_path, member_removal_path,
};
pub use refusal::{
    GATEWAY_REFUSAL_ERRCODE_PREFIX, GATEWAY_REFUSAL_HEADER, GatewayRefusal, HOP_LIMIT_ERRCODE,
};
pub use shared_bot::{BotAccess, NewSharedBot, SharedBot, SharedBotsState, SharedBotsView};
pub use view_model::{
    FamilyAccess, FamilyFailure, FamilyProblem, FamilyViewModel, JoinRequestRow,
    JoinRequestsViewModel, MemberCode, MemberInviteView, MemberRow, MembersViewModel,
};
