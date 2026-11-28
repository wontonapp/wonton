//! 工具替谁办事（ADR-0113 第 5 节）：私人 Bot 是归属人，共享 Bot 是这一轮说话的人。`person` 只能写这个人
//! （用户名或完整 MXID 都认），写了别人回 `permission_denied`。说不出替谁办事时同样拒，不猜成归属人或主人：
//! 缺说话人就拿到别人的数据，等于身份升级（同 [`Bus::ai_cap`] 的理由）。

use wonton_protocol::{BotKeeper, MemberStatus};

use super::super::audience::localpart;
use super::super::{Bus, Caller, ToolFailure, TurnSender};
use crate::device::{Device, Devices};

/// 这次工具作用的人。`person` 是媒体库里记人的完整 MXID。
pub(super) struct Subject {
    pub(super) username: String,
    pub(super) person: String,
    owner: bool,
}

impl Subject {
    /// 这个人名下的已配对设备；主人的设备在登记表里不记人。
    pub(super) fn devices(&self, devices: &Devices) -> Vec<Device> {
        if self.owner {
            devices
                .list()
                .into_iter()
                .filter(|device| device.user.is_none())
                .collect()
        } else {
            devices.of_user(&self.username)
        }
    }
}

pub(super) fn subject(
    bus: &Bus,
    caller: &Caller,
    asked: Option<&str>,
) -> Result<Subject, ToolFailure> {
    let Some(bot) = &caller.bot else {
        return Err(denied(
            "媒体工具只给 Bot 用：这次调用没有 Bot 身份，说不出替谁办事",
        ));
    };
    let username = match &bot.keeper {
        BotKeeper::User { username } => username.clone(),
        BotKeeper::Household => speaker(bus, caller)?,
        BotKeeper::Other => return Err(denied("认不出这个 Bot 归谁")),
    };
    let person = bus.user_id_of(&username)?;
    if let Some(asked) = asked.filter(|asked| !asked.is_empty())
        && asked != username
        && asked != person
    {
        return Err(denied(format!("只能替 {person} 查和操作，不能替 {asked}")));
    }
    let owner = username == bus.plugins.owner_name();
    Ok(Subject {
        username,
        person,
        owner,
    })
}

/// 共享 Bot 这一轮说话的人：要是本服务器上还在家里的人。
fn speaker(bus: &Bus, caller: &Caller) -> Result<String, ToolFailure> {
    let user_id = match &caller.sender {
        Some(TurnSender::Local(user_id)) => user_id,
        Some(TurnSender::Unknown) => return Err(denied("认不出这一轮说话的人")),
        None => {
            return Err(denied(
                "这次调用没说替谁办事（没带说话人），共享 Bot 不知道该替谁",
            ));
        }
    };
    let username =
        localpart(user_id).ok_or_else(|| denied(format!("{user_id} 不是 Matrix 用户 ID")))?;
    if bus.user_id_of(username)? != *user_id {
        return Err(denied(format!("{user_id} 不是这台服务器上的人")));
    }
    let known = username == bus.plugins.owner_name()
        || bus
            .people()
            .ok()
            .and_then(|people| people.members.get(username))
            .is_some_and(|member| {
                matches!(member.status, MemberStatus::Invited | MemberStatus::Joined)
            });
    if !known {
        return Err(denied(format!("{user_id} 不在这个家里")));
    }
    Ok(username.to_owned())
}

fn denied(message: impl Into<String>) -> ToolFailure {
    ToolFailure::new("permission_denied", message)
}

impl Bus {
    /// 本服务器上这个用户名的完整 MXID（媒体库按它记人）。
    fn user_id_of(&self, username: &str) -> Result<String, ToolFailure> {
        let people = self
            .people()
            .map_err(|e| ToolFailure::new("service_unavailable", e))?;
        Ok(format!("@{username}:{}", people.server_name))
    }
}
