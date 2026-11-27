use serde::{Deserialize, Serialize};

/// 发起私聊时能直接选的人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChatContact {
    /// Matrix 用户 ID。Bot 记录还没建出来（`BotMissing`）时为空：这时只能看见，还不能发起。
    pub user_id: String,
    pub name: String,
    /// 对方是 Bot 时的精确 Bot 身份；普通人联系人为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub bot: Option<crate::BotRef>,
    /// 选这个人开新旁聊时在哪个房间开（ADR-0093 §4）：和这个人已有的、有支持声明的私聊。
    /// 为空时壳走 `StartDirectChat` 先建私聊。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_chat_room_id: Option<String>,
    /// 对方是 Bot 时当前请求人的 [`BotAccess`]；普通人联系人默认 [`BotAccess::Unknown`]。
    /// 未知时不写进 wire，旧解析端缺字段按默认值处理。
    #[serde(default, skip_serializing_if = "is_unknown_access")]
    pub access: crate::BotAccess,
}

fn is_unknown_access(access: &crate::BotAccess) -> bool {
    *access == crate::BotAccess::Unknown
}
