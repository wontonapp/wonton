use serde::{Deserialize, Serialize};

use crate::BotRef;

/// Bot 联系人操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BotContactAction {
    /// 打开某个 Bot 的联系人或下拉刷新（`GET`
    /// [`bot_contacts_path`](crate::bot_contacts_path)），之后的结果都是这个 Bot 的。
    RefreshBotContacts { bot: BotRef },
    /// 请另一个 Bot（本机的，或配对过的服务器上的）做这个 Bot 的联系人，等它的归属人同意。
    RequestBotContact { bot: BotRef, user_id: String },
    /// 请配对过的服务器上的一个人做这个 Bot 的联系人：Bot 给他发带标记的私聊邀请，他接受才算数，之前一直是“等对方接受”。
    RequestPersonContact { bot: BotRef, user_id: String },
    /// 同意对方发来的请求（[`ContactState::Incoming`](crate::ContactState::Incoming)）。
    ApproveBotContact { bot: BotRef, user_id: String },
    /// 拒绝对方发来的请求。
    RejectBotContact { bot: BotRef, user_id: String },
    /// 删掉联系人、取消自己发出的请求，或清掉“对方拒绝了”“没有这个人”的行；同时通知对方。
    RemoveBotContact { bot: BotRef, user_id: String },
}
