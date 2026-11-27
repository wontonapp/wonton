use serde::{Deserialize, Serialize};

/// 一个 Bot：插件 ID 加插件里的 Bot 名（插件规范的 `bots`）。私人 Bot 的标识以 [`Member::private_bot`](super::Member::private_bot) 为准。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotRef {
    pub plugin_id: String,
    pub bot: String,
}

/// 本人私人 Bot 查询的成功响应，绑定当前认证人的完整 Matrix 用户 ID。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnPrivateBotResponse {
    pub requester_user_id: String,
    pub result: OwnPrivateBotResult,
}

/// 本人身份的查询结果，不代表 Bot 可用或房间可发送。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OwnPrivateBotResult {
    NotApplicable,
    Absent,
    Ready(PrivateBotIdentity),
    #[serde(other)]
    Other,
}

/// 已核实的本人私人 Bot 身份和同一记录的主房间。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PrivateBotIdentityFields")]
pub struct PrivateBotIdentity {
    pub bot: BotRef,
    pub user_id: String,
    pub home_room: Option<String>,
    pub room_problem: Option<PrivateBotRoomProblem>,
}

#[derive(Deserialize)]
struct PrivateBotIdentityFields {
    bot: BotRef,
    user_id: String,
    home_room: Option<String>,
    room_problem: Option<PrivateBotRoomProblem>,
}

impl TryFrom<PrivateBotIdentityFields> for PrivateBotIdentity {
    type Error = &'static str;

    fn try_from(fields: PrivateBotIdentityFields) -> Result<Self, Self::Error> {
        if fields.home_room.is_some() && fields.room_problem.is_some() {
            return Err("private Bot room and room problem conflict");
        }
        Ok(Self {
            bot: fields.bot,
            user_id: fields.user_id,
            home_room: fields.home_room,
            room_problem: fields.room_problem,
        })
    }
}

/// 主房间记录不能用作房间目标的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PrivateBotRoomProblem {
    /// 主房间记录不是合法的房间 ID。
    Invalid,
    /// Bot 记着主房间，但它还没加入那个房间：等它进房间再发。服务端暂时还不产出，
    /// 客户端先按 `view.rooms` 的成员状态判。
    NotJoined,
    /// Bot 还没有主房间记录：可以给它建一个私聊。
    Missing,
    #[serde(other)]
    Other,
}

/// Bot 听谁的、谁管它。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BotKeeper {
    /// 私人 Bot：归这个人。
    User { username: String },
    /// 共享 Bot：归全家，主人管。
    Household,
    #[serde(other)]
    Other,
}
