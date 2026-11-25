//! Bot 的 Matrix 身份（插件规范的“Matrix Bot”）：分配用户名，代 Bot 登录一台设备，设置显示名和头像，
//! 由主人建一间私聊作为主房间并把 Bot 拉进来，插件装好后再交给主人（[`Matrix::hand_over`]）。
//!
//! 账号和主房间建好后一直沿用（重装、安装失败重试都一样），只有删除数据卸载时才停用账号；设备每次安装新建、
//! 卸载时吊销。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::admin::{ADMIN_LOCALPART, SERVER_USER_LOCALPART};
use super::api::{Login, MatrixError};
use super::bot_home::next_reservation;
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
        self.provision_bot_checkpointed(spec, |_| Ok(())).await
    }

    pub async fn provision_bot_checkpointed(
        &self,
        spec: &BotSpec<'_>,
        mut checkpoint: impl FnMut(&ProvisionedBot) -> Result<(), MatrixError>,
    ) -> Result<ProvisionedBot, MatrixError> {
        let login = self.login_bot(spec).await?;
        let mut provisioned = ProvisionedBot {
            account: BotAccount {
                user_id: login.user_id.clone(),
                home_room: spec
                    .previous
                    .map(|account| account.home_room.clone())
                    .unwrap_or_default(),
            },
            device_id: login.device_id.clone(),
            access_token: login.access_token.clone(),
            owner: self.user_id(spec.owner),
        };
        let result = async {
            checkpoint(&provisioned)?;
            provisioned.account.home_room = self.prepare_bot(spec, &login).await?;
            if let Err(error) = checkpoint(&provisioned) {
                if spec
                    .previous
                    .is_none_or(|old| old.home_room != provisioned.account.home_room)
                {
                    self.withdraw(&provisioned, &Languages::default()).await?;
                }
                return Err(error);
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            self.delete_device(&login.user_id, &login.device_id).await?;
            return Err(error);
        }
        Ok(provisioned)
    }

    /// 上次的账号直接登录（账号已停用时报错，不悄悄换一个）。没有时按顺序分配一个还没人用过的用户名：JWT 登录对
    /// 已有的账号是直接登录进去，所以已经存在的账号（家人、别的插件的 Bot、停用的账号）一律跳过，不接管。只有这个
    /// 插件记录在案的 Bot（`previous`）能登录已有的账号。
    async fn login_bot(&self, spec: &BotSpec<'_>) -> Result<Login, MatrixError> {
        let account = self.reserve_bot_account(spec).await?;
        self.login(localpart(&account.user_id)?, spec.device_name)
            .await
    }

    /// Select an unclaimed identity without creating it. The caller persists this reservation
    /// before JWT login so a failed room/profile operation cannot allocate another account.
    pub async fn reserve_bot_account(&self, spec: &BotSpec<'_>) -> Result<BotAccount, MatrixError> {
        if let Some(previous) = spec.previous {
            return Ok(previous.clone());
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
            return Ok(BotAccount {
                user_id: self.user_id(&candidate),
                home_room: String::new(),
            });
        }
        Err(MatrixError::Invalid(format!(
            "分配 Bot 用户名失败：{0} 到 {0}-{MAX_CANDIDATES} 都已经有人用了",
            spec.localpart
        )))
    }

    /// 设置显示名和头像，返回主房间：上次的主房间还能用（见 [`reusable`]）就沿用，否则由主人新建一间。
    async fn prepare_bot(&self, spec: &BotSpec<'_>, login: &Login) -> Result<String, MatrixError> {
        let bot = self.session(login);
        if spec
            .previous
            .is_none_or(|account| account.home_room.is_empty())
        {
            bot.set_display_name(spec.display_name).await?;
            if let Some(avatar) = &spec.avatar {
                bot.set_avatar(avatar.content_type, avatar.filename, avatar.data.to_vec())
                    .await?;
            }
        }
        let owner = self.user_id(spec.owner);
        let handed_over = self.handed_over(spec, &bot).await?;
        if let Some(previous) = spec.previous
            && !previous.home_room.is_empty()
            && reusable(&bot, &previous.home_room, &owner, handed_over).await?
        {
            return Ok(previous.home_room.clone());
        }
        if let Some(room) = bot.recover_bot_home(&owner, handed_over).await? {
            return Ok(room);
        }
        self.create_bot_home(&bot, spec.owner, &owner).await
    }

    /// 主人建房、主人邀请、Bot 接受：Bot 的进房事件发送人是主人。插件适配器只收允许名单里的邀请人
    /// （名单是归属人和联系人，Bot 自己不在里面），所以不能自己邀请自己。
    pub(super) async fn create_bot_home(
        &self,
        bot: &Session<'_>,
        owner: &str,
        owner_user: &str,
    ) -> Result<String, MatrixError> {
        let reservation = next_reservation();
        self.as_owner(owner, async |me: &Session<'_>| {
            // `joined_rooms` 先拍个快照：`createRoom` 的响应丢掉时，只有拿它做差才能认出这次新建的房。
            let before = me.joined_rooms().await?;
            let room = match me
                .create_bot_home(owner_user, bot.user_id(), reservation)
                .await
            {
                Ok(room) => room,
                Err(error @ MatrixError::Unavailable { .. }) => {
                    // 服务端建房了、主人是成员，客户端没拿到 room_id：原来的清理块碰不到它，主人会一直在房里。
                    // 按这次建房号找出来退出，不让死房留在主人的列表里。
                    if let Err(cleanup) = me
                        .leave_new_bot_homes(owner_user, &before, reservation)
                        .await
                    {
                        tracing::warn!(error = %cleanup, "清理建房失败留下的主房间失败");
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            // 建房之后的步骤失败了：主人这时就在房间里，房间又是刚建的、除了这次调用谁都不认识，不退出就永远
            // 留在主人的聊天列表里，重试还会另建一间。退出成功、房间就只在服务器的库里，留不留都不影响主人。
            let done = async {
                me.invite(&room, bot.user_id()).await?;
                bot.join(&room).await?;
                // 主人退出，房间就不在主人的聊天列表里了（列表只要没退出的房间），交出去（`hand_over`）才回来。
                me.leave_room(&room, &Languages::default()).await
            }
            .await;
            if let Err(error) = done {
                // 退出也失败时照样报原错误：那是装不上的原因。房间留在列表里，主人看得见、删得掉。
                me.leave_room(&room, &Languages::default()).await.ok();
                return Err(error);
            }
            Ok(room)
        })
        .await
    }

    /// 把主房间交给主人：把主人请进房间，房间记进双方的 `m.direct`。主人建房后已经退出（见
    /// [`Matrix::create_bot_home`]），旧版本的房间里主人可能还没被邀请，按成员状态补上；已经交出过（重装）时
    /// 什么也不做，返回 false。中途失败时撤回（[`Self::withdraw`]）再返回错误。
    pub async fn hand_over(
        &self,
        owner: &str,
        bot: &ProvisionedBot,
        langs: &Languages,
    ) -> Result<bool, MatrixError> {
        let session = Session::new(&self.api, &bot.account.user_id, &bot.access_token);
        let room = &bot.account.home_room;
        let handed_over = self
            .as_owner(owner, async |me: &Session<'_>| {
                // 主人的 `m.direct` 里已经有这个房间：上次已经交出过了。放最后写，拿它当交完的标记。
                if me.has_direct(&bot.account.user_id, room).await? {
                    return Ok(false);
                }
                // 成员状态用 Bot 的会话读：主人退出过就未必还能读房间状态。旧版本建的房间主人可能还没被邀请，
                // 建房后退出和这些一样，邀请一次。
                match session.membership(room, &bot.owner).await?.as_deref() {
                    Some("join") => {}
                    Some("invite") => me.join(room).await?,
                    _ => {
                        session.invite(room, &bot.owner).await?;
                        me.join(room).await?;
                    }
                }
                session.add_direct(&bot.owner, room).await?;
                me.add_direct(&bot.account.user_id, room).await?;
                Ok(true)
            })
            .await;
        match handed_over {
            Ok(true) => Ok(true),
            Ok(false) => Ok(false),
            Err(e) => {
                if let Err(cleanup) = self.withdraw(bot, langs).await {
                    tracing::error!(%room, error = %cleanup, "撤回没交给主人的主房间失败");
                }
                Err(e)
            }
        }
    }

    /// 收回交给主人的主房间（安装在交出之后失败了）：主人退出，Bot 也退出。要在吊销 Bot 的设备之前做。
    ///
    /// 主人是建房人，Bot 虽然也是 creator 却踢不动另一个 creator（Tuwunel 回 403 `M_FORBIDDEN`），所以让主人自己退；
    /// 房间还是邀请态、或者旧版本由 Bot 建的房间里主人不是 creator 时，退出也一样。
    pub async fn withdraw(
        &self,
        bot: &ProvisionedBot,
        langs: &Languages,
    ) -> Result<(), MatrixError> {
        let session = Session::new(&self.api, &bot.account.user_id, &bot.access_token);
        let room = &bot.account.home_room;
        let owner = localpart(&bot.owner)?;
        self.as_owner(owner, async |me: &Session<'_>| {
            // 成员状态用 Bot 的会话读：主人退出过就未必还能读房间状态（重试撤回时他已经不在里面）。
            if matches!(
                session.membership(room, &bot.owner).await?.as_deref(),
                Some("join" | "invite")
            ) {
                me.leave_room(room, langs).await?;
            }
            // `m.direct` 是交出去的标记：撤回了就一起去掉，重试时不会再当成交过。
            me.remove_direct(&bot.account.user_id, room).await?;
            Ok(())
        })
        .await?;
        session.retire_direct(&bot.owner, room).await;
        Ok(())
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

    /// 上次的主房间交出去过没有：主人的 `m.direct` 是交完才写的标记，主人退出过的房间只有没交出去的才还能用。
    async fn handed_over(
        &self,
        spec: &BotSpec<'_>,
        bot: &Session<'_>,
    ) -> Result<bool, MatrixError> {
        match spec.previous {
            Some(previous) if !previous.home_room.is_empty() => {
                let bot_user = bot.user_id();
                self.as_owner(spec.owner, async |me: &Session<'_>| {
                    me.has_direct(bot_user, &previous.home_room).await
                })
                .await
            }
            _ => Ok(false),
        }
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

/// 上次的主房间能不能接着用：Bot 在里面，主人还在里面、还等着接受邀请，或者建房后刚退出。退出过的是不是主人自己
/// 结束的私聊看 `handed_over`：交出过（`m.direct` 里有）就不再用，没交出过是建房后的正常状态（见
/// [`Matrix::create_bot_home`]）。捡回（[`Session::recover_bot_home`]）传同一个值。Bot 已经不在房间里时 Tuwunel 回
/// `M_FORBIDDEN`。
pub(super) async fn reusable(
    bot: &Session<'_>,
    room: &str,
    owner: &str,
    handed_over: bool,
) -> Result<bool, MatrixError> {
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
    match bot.membership(room, owner).await?.as_deref() {
        None | Some("invite") => Ok(true),
        Some("leave") => Ok(!handed_over),
        Some(_) => Ok(false),
    }
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
