//! Bot 给一个人发联系人邀请（ADR-0050）：核心代 Bot 登录一台设备，建带 [`wonton_protocol::BOT_CONTACT_CREATE_KEY`]
//! 的私聊并邀请他，之后用同一台设备查他接受没有、撤回邀请。
//!
//! 设备留着不退出：接受、拒绝只能从房间里看出来，要隔一会儿查一次；每次查都新登录一台再退出，Bot 的设备列表就一直在变，
//! 和它一起在加密房间里的人的客户端也跟着刷新设备。设备的令牌存在 `core/contact-sessions.json`（0600），失效了重新登录。

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use wonton_protocol::ContactError;

use super::store::StoreError;
use super::{ContactFailure, InviteOutcome, InviteeState, LocalBot, PersonInvites};
use crate::matrix::{Login, Matrix, MatrixError};

const DEVICE_NAME: &str = "Wonton contacts";

#[derive(Clone, Serialize, Deserialize)]
struct StoredLogin {
    user_id: String,
    device_id: String,
    access_token: String,
}

pub struct MatrixInvites {
    matrix: Arc<Matrix>,
    path: PathBuf,
    sessions: tokio::sync::Mutex<BTreeMap<String, StoredLogin>>,
}

impl MatrixInvites {
    pub fn open(path: PathBuf, matrix: Arc<Matrix>) -> Result<Self, StoreError> {
        let sessions = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| StoreError::Corrupt {
                path: path.clone(),
                message: e.to_string(),
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(source) => return Err(StoreError::Io { path, source }),
        };
        Ok(Self {
            matrix,
            path,
            sessions: tokio::sync::Mutex::new(sessions),
        })
    }

    /// 这个 Bot 的设备；`fresh` 时丢掉存着的、重新登录（令牌失效了）。
    async fn login(&self, bot: &LocalBot, fresh: bool) -> Result<Login, ContactFailure> {
        let mut sessions = self.sessions.lock().await;
        if !fresh && let Some(stored) = sessions.get(&bot.user_id) {
            return Ok(Login {
                user_id: stored.user_id.clone(),
                device_id: stored.device_id.clone(),
                access_token: stored.access_token.clone(),
            });
        }
        let localpart = bot
            .user_id
            .strip_prefix('@')
            .and_then(|rest| rest.split_once(':'))
            .map(|(local, _)| local)
            .ok_or_else(|| {
                ContactFailure::new(
                    ContactError::Upstream,
                    format!("Bot 的用户 ID {} 不对", bot.user_id),
                )
            })?;
        let login = self
            .matrix
            .login(localpart, DEVICE_NAME)
            .await
            .map_err(|e| upstream(&e))?;
        sessions.insert(
            bot.user_id.clone(),
            StoredLogin {
                user_id: login.user_id.clone(),
                device_id: login.device_id.clone(),
                access_token: login.access_token.clone(),
            },
        );
        if let Err(e) = self.save(&sessions) {
            tracing::error!(error = %e, "联系人设备的令牌写不进去，下次重新登录");
        }
        Ok(login)
    }

    fn save(&self, sessions: &BTreeMap<String, StoredLogin>) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(sessions).expect("令牌表总能序列化");
        let tmp = self.path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &self.path)
    }

    /// 令牌失效时要重新登录再调一次。
    fn stale(result: &Result<impl Sized, MatrixError>) -> bool {
        matches!(result, Err(e) if e.errcode() == "M_UNKNOWN_TOKEN")
    }
}

fn upstream(error: &MatrixError) -> ContactFailure {
    ContactFailure::new(ContactError::Upstream, error.to_string())
}

/// 邀请的错误分类（ADR-0050）：403 是对方拒绝收（MSC4155 的拒绝名单），查无此人是“没有这个人”；对方服务器的问题
/// 是连不上。本机 Tuwunel 不可达是上游出错。
fn invite_outcome(
    user: &str,
    own_server: &str,
    error: MatrixError,
) -> Result<InviteOutcome, ContactFailure> {
    let remote = super::server_of(user) != Some(own_server);
    match &error {
        MatrixError::Rejected { status: 403, .. } => Ok(InviteOutcome::Declined),
        MatrixError::Rejected { status: 404, .. } => Ok(InviteOutcome::NotFound),
        MatrixError::Rejected { errcode, .. } if errcode == "M_NOT_FOUND" => {
            Ok(InviteOutcome::NotFound)
        }
        MatrixError::Rejected { status, .. } if remote && *status >= 500 => Err(
            ContactFailure::new(ContactError::PeerUnreachable, error.to_string()),
        ),
        _ => Err(upstream(&error)),
    }
}

impl PersonInvites for MatrixInvites {
    fn invite<'a>(
        &'a self,
        bot: &'a LocalBot,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<InviteOutcome, ContactFailure>> {
        Box::pin(async move {
            let login = self.login(bot, false).await?;
            let mut result = self.matrix.session(&login).invite_as_contact(user_id).await;
            if Self::stale(&result) {
                let login = self.login(bot, true).await?;
                result = self.matrix.session(&login).invite_as_contact(user_id).await;
            }
            match result {
                Ok(room_id) => Ok(InviteOutcome::Sent { room_id }),
                Err(e) => invite_outcome(user_id, self.matrix.server_name(), e),
            }
        })
    }

    fn state<'a>(
        &'a self,
        bot: &'a LocalBot,
        room_id: &'a str,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<InviteeState, ContactFailure>> {
        Box::pin(async move {
            let login = self.login(bot, false).await?;
            let mut result = self
                .matrix
                .session(&login)
                .membership(room_id, user_id)
                .await;
            if Self::stale(&result) {
                let login = self.login(bot, true).await?;
                result = self
                    .matrix
                    .session(&login)
                    .membership(room_id, user_id)
                    .await;
            }
            let membership = result.map_err(|e| upstream(&e))?;
            Ok(match membership.as_deref() {
                Some("join") => InviteeState::Joined,
                Some("leave" | "ban") => InviteeState::Left,
                // 还在邀请，或者房间里还看不到他的成员事件：照实当成还在等。
                _ => InviteeState::Invited,
            })
        })
    }

    fn withdraw<'a>(
        &'a self,
        bot: &'a LocalBot,
        room_id: &'a str,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ContactFailure>> {
        Box::pin(async move {
            let login = self.login(bot, false).await?;
            let mut result = self
                .matrix
                .session(&login)
                .withdraw_invite(room_id, user_id)
                .await;
            if Self::stale(&result) {
                let login = self.login(bot, true).await?;
                result = self
                    .matrix
                    .session(&login)
                    .withdraw_invite(room_id, user_id)
                    .await;
            }
            result.map_err(|e| upstream(&e))
        })
    }
}
