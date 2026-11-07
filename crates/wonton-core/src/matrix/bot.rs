//! Bot 的 Matrix 身份（插件规范的“Matrix Bot”）：分配用户名，代 Bot 登录一台设备，设置显示名和头像，
//! 建好和主人的加密私聊作为主房间，插件装好后再交给主人（[`Matrix::hand_over`]）。
//!
//! 账号和主房间建好后一直沿用（重装、安装失败重试都一样），只有删除数据卸载时才停用账号；设备每次安装新建、
//! 卸载时吊销。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::admin::{ADMIN_LOCALPART, SERVER_USER_LOCALPART};
use super::api::{Login, MatrixError};
use super::{Matrix, Session};
use crate::plugin::languages::Languages;

/// 换用户名的次数上限：`<名字>`、`<名字>-2` …… `<名字>-20`。
const MAX_CANDIDATES: u32 = 20;

/// 代主人加入主房间时临时登录的设备名，做完就退出登录。
const OWNER_DEVICE: &str = "Wonton 核心（临时）";

/// 要建的 Bot。
pub struct BotSpec<'a> {
    /// 首选的用户名；被占用或账号已停用时依次试 `<名字>-2`、`<名字>-3` ……
    pub localpart: &'a str,
    /// 别的插件的 Bot 已经用掉的用户名。
    pub taken: &'a BTreeSet<String>,
    /// 这个插件上次的 Bot：沿用它的账号和主房间。
    pub previous: Option<&'a BotAccount>,
    pub device_name: &'a str,
    pub display_name: &'a str,
    pub avatar: Option<Avatar<'a>>,
    /// 主人的用户名（localpart）。
    pub owner: &'a str,
}

pub struct Avatar<'a> {
    pub content_type: &'a str,
    pub filename: &'a str,
    pub data: &'a [u8],
}

/// Bot 的账号和主房间：核心记下来，重装时沿用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotAccount {
    pub user_id: String,
    pub home_room: String,
}

/// 建好的 Bot：账号、主房间，和这次登录的设备。`Debug` 不打印令牌。
#[derive(Clone, PartialEq, Eq)]
pub struct ProvisionedBot {
    pub account: BotAccount,
    pub device_id: String,
    pub access_token: String,
    /// 主人的 Matrix 用户 ID。
    pub owner: String,
}

impl std::fmt::Debug for ProvisionedBot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvisionedBot")
            .field("account", &self.account)
            .field("device_id", &self.device_id)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl Matrix {
    /// 代 Bot 登录一台新设备，设置资料，准备好主房间。中途失败时吊销这台设备再返回错误。
    pub async fn provision_bot(&self, spec: &BotSpec<'_>) -> Result<ProvisionedBot, MatrixError> {
        let login = self.login_bot(spec).await?;
        match self.prepare_bot(spec, &login).await {
            Ok(home_room) => Ok(ProvisionedBot {
                account: BotAccount {
                    user_id: login.user_id,
                    home_room,
                },
                device_id: login.device_id,
                access_token: login.access_token,
                owner: self.user_id(spec.owner),
            }),
            Err(e) => {
                if let Err(cleanup) = self.delete_device(&login.user_id, &login.device_id).await {
                    tracing::error!(user = %login.user_id, error = %cleanup, "吊销没建好的 Bot 设备失败");
                }
                Err(e)
            }
        }
    }

    /// 上次的账号直接登录（账号已停用时报错，不悄悄换一个）。没有时按顺序分配一个还没人用过的用户名：JWT 登录对
    /// 已有的账号是直接登录进去，所以已经存在的账号（家人、别的插件的 Bot、停用的账号）一律跳过，不接管。只有这个
    /// 插件记录在案的 Bot（`previous`）能登录已有的账号。
    async fn login_bot(&self, spec: &BotSpec<'_>) -> Result<Login, MatrixError> {
        if let Some(previous) = spec.previous {
            let localpart = localpart(&previous.user_id)?;
            return self.login(localpart, spec.device_name).await;
        }
        let reserved = [ADMIN_LOCALPART, SERVER_USER_LOCALPART, spec.owner];
        for n in 1..=MAX_CANDIDATES {
            let candidate = match n {
                1 => spec.localpart.to_owned(),
                n => format!("{}-{n}", spec.localpart),
            };
            if reserved.contains(&candidate.as_str())
                || spec.taken.contains(&candidate)
                || self.user_exists(&self.user_id(&candidate)).await?
            {
                continue;
            }
            return self.login(&candidate, spec.device_name).await;
        }
        Err(MatrixError::Invalid(format!(
            "分配 Bot 用户名失败：{0} 到 {0}-{MAX_CANDIDATES} 都已经有人用了",
            spec.localpart
        )))
    }

    /// 设置显示名和头像，返回主房间：上次的主房间还能用（见 [`reusable`]）就沿用，否则新建一个，先只有 Bot 在里面。
    async fn prepare_bot(&self, spec: &BotSpec<'_>, login: &Login) -> Result<String, MatrixError> {
        let bot = self.session(login);
        if spec.previous.is_none() {
            bot.set_display_name(spec.display_name).await?;
            if let Some(avatar) = &spec.avatar {
                bot.set_avatar(avatar.content_type, avatar.filename, avatar.data.to_vec())
                    .await?;
            }
        }
        let owner = self.user_id(spec.owner);
        if let Some(previous) = spec.previous
            && reusable(&bot, &previous.home_room, &owner).await?
        {
            return Ok(previous.home_room.clone());
        }
        bot.create_direct_room(&owner).await
    }

    /// 把主房间交给主人：Bot 邀请主人、记进自己的 `m.direct`，核心代主人加入、记进主人的 `m.direct`。主人已经在
    /// 里面（重装）时什么也不做，返回 false。中途失败时撤回（[`Session::abandon`]）再返回错误。
    pub async fn hand_over(
        &self,
        owner: &str,
        bot: &ProvisionedBot,
        langs: &Languages,
    ) -> Result<bool, MatrixError> {
        let session = Session::new(&self.api, &bot.account.user_id, &bot.access_token);
        let room = &bot.account.home_room;
        if session.joined_members(room).await?.contains(&bot.owner) {
            return Ok(false);
        }
        // 先代主人登录再邀请：主人的账号在第一次 JWT 登录时才建出来，邀请发给已有的账号。
        let handed_over = self
            .as_owner(owner, async |me: &Session<'_>| {
                session.invite(room, &bot.owner).await?;
                session.add_direct(&bot.owner, room).await?;
                me.join(room).await?;
                me.add_direct(&bot.account.user_id, room).await
            })
            .await;
        if let Err(e) = handed_over {
            if let Err(cleanup) = session.abandon(room, &bot.owner, langs).await {
                tracing::error!(%room, error = %cleanup, "撤回没交给主人的主房间失败");
            }
            return Err(e);
        }
        Ok(true)
    }

    /// 收回交给主人的主房间（安装在交出之后失败了）：主人退出，Bot 也退出。要在吊销 Bot 的设备之前做。
    pub async fn withdraw(
        &self,
        bot: &ProvisionedBot,
        langs: &Languages,
    ) -> Result<(), MatrixError> {
        Session::new(&self.api, &bot.account.user_id, &bot.access_token)
            .abandon(&bot.account.home_room, &bot.owner, langs)
            .await
    }

    /// 代主人加入房间，并写进主人的 `m.direct`。
    pub(super) async fn owner_joins(
        &self,
        owner: &str,
        bot: &str,
        room: &str,
    ) -> Result<(), MatrixError> {
        self.as_owner(owner, async |me: &Session<'_>| {
            me.join(room).await?;
            me.add_direct(bot, room).await
        })
        .await
    }

    /// 用 JWT 临时登录主人的一台设备做 `work`，做完就退出登录，这台设备随之删除。
    async fn as_owner<T>(
        &self,
        owner: &str,
        work: impl AsyncFnOnce(&Session<'_>) -> Result<T, MatrixError>,
    ) -> Result<T, MatrixError> {
        let login = self.login(owner, OWNER_DEVICE).await?;
        let session = self.session(&login);
        let done = work(&session).await;
        let logout = session.logout().await;
        done.and_then(|value| logout.map(|()| value))
    }
}

/// 上次的主房间能不能接着用：Bot 还在里面，主人也在，或者还没交给过主人（上次安装没装完，主人从来没进过、或者只
/// 收到了邀请）。主人退出过就不用了，不往没人看的房间里发。Bot 已经不在房间里时 Tuwunel 回 `M_FORBIDDEN`。
async fn reusable(bot: &Session<'_>, room: &str, owner: &str) -> Result<bool, MatrixError> {
    let members = match bot.joined_members(room).await {
        Ok(members) => members,
        Err(e) if matches!(e.errcode(), "M_FORBIDDEN" | "M_NOT_FOUND") => return Ok(false),
        Err(e) => return Err(e),
    };
    if !members.iter().any(|m| m == bot.user_id()) {
        return Ok(false);
    }
    if members.iter().any(|m| m == owner) {
        return Ok(true);
    }
    let owner = bot.membership(room, owner).await?;
    Ok(matches!(owner.as_deref(), None | Some("invite")))
}

/// `@名字:服务器` 里的名字。
fn localpart(user_id: &str) -> Result<&str, MatrixError> {
    user_id
        .strip_prefix('@')
        .and_then(|rest| rest.split_once(':'))
        .map(|(localpart, _)| localpart)
        .filter(|localpart| !localpart.is_empty())
        .ok_or_else(|| MatrixError::Invalid(format!("Bot 的用户 ID `{user_id}` 不合法")))
}
