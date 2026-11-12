//! 联系人逻辑和外面的接口：本机 Bot 的名录、两台核心之间的通道、Bot 发给人的邀请、名单变了以后的副作用。
//! 逻辑只经这几个 trait 碰外面，测试里换成内存里的两台服务器。

use futures_util::future::BoxFuture;
use wonton_protocol::{
    BotKeeper, BotRef, ContactError, ContactEvent, ContactRequestPush, PeerStatus,
};

/// 联系人操作失败：类别给 App，`detail` 是给日志和“详情”的原因（核心写，不翻译）。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?}: {detail}")]
pub struct ContactFailure {
    pub kind: ContactError,
    pub detail: String,
}

impl ContactFailure {
    pub fn new(kind: ContactError, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

/// 本机的一个 Bot。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalBot {
    pub bot: BotRef,
    pub user_id: String,
    pub display_name: Option<String>,
    pub keeper: BotKeeper,
    /// 管它的人的用户名：私人 Bot 是归属人，共享 Bot 是主人。
    pub keeper_username: String,
    /// 私人 Bot 的归属人的显示名；共享 Bot 为空。
    pub keeper_display_name: Option<String>,
}

/// 本机 Bot 的名录。读不出（记录损坏）时报错，不当成没有这个 Bot。
pub trait Directory: Send + Sync {
    fn bot(&self, bot: &BotRef) -> Result<Option<LocalBot>, String>;
    fn bot_by_user(&self, user_id: &str) -> Result<Option<LocalBot>, String>;
}

/// 对方服务器在配对表里的样子。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerReach {
    /// 没配对，或配对已经移除。
    NotPaired,
    /// 已配对；状态是转发组件给的（ADR-0050 的状态表）。
    Paired(PeerStatus),
}

/// 两台核心之间交换 [`ContactEvent`] 的通道。收到的事件由通道核对来源后交给
/// [`super::Contacts::receive`]。
pub trait PeerLink: Send + Sync {
    fn reach(&self, server_name: &str) -> PeerReach;
    /// 发给 `server_name` 的核心。发不出去（房间还没建好、对方连不上、Tuwunel 出错）时报错，什么都不算发出。
    fn send(
        &self,
        server_name: &str,
        event: ContactEvent,
    ) -> BoxFuture<'_, Result<(), ContactFailure>>;
}

/// Bot 给一个人发联系人邀请的结果（ADR-0050）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InviteOutcome {
    /// 邀请发到了；等他接受。
    Sent { room_id: String },
    /// 对方拒绝收（MSC4155 屏蔽了这个 Bot 或这台服务器，邀请回 403）。
    Declined,
    /// 对方服务器上没有这个人。
    NotFound,
}

/// 被邀请的人现在怎样了。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteeState {
    /// 还没处理（包括忽略）。
    Invited,
    Joined,
    /// 拒绝了邀请，或者被禁止进入。
    Left,
}

/// 以 Bot 的身份发、查、撤回私聊邀请。
pub trait PersonInvites: Send + Sync {
    fn invite<'a>(
        &'a self,
        bot: &'a LocalBot,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<InviteOutcome, ContactFailure>>;
    fn state<'a>(
        &'a self,
        bot: &'a LocalBot,
        room_id: &'a str,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<InviteeState, ContactFailure>>;
    fn withdraw<'a>(
        &'a self,
        bot: &'a LocalBot,
        room_id: &'a str,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ContactFailure>>;
}

/// 联系人变了以后对外的事。都在联系人的记录写好之后调，失败由实现方记日志：记录是准的，下次重建照记录来。
pub trait Effects: Send + Sync {
    /// 这个 Bot 的已成立联系人变了：重建它的服务，让 `WONTON_MATRIX_ALLOWED_USERS` 跟上。
    fn allowed_users_changed(&self, bot: &BotRef);
    /// 推送一条新的联系人请求给 `keeper_username` 的设备。
    fn notify_request(&self, keeper_username: &str, push: ContactRequestPush);
}
