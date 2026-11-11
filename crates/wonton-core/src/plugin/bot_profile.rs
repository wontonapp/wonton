//! Bot 资料的两条写入路径共用验证与串行化；Matrix 是资料的事实源（ADR-0044）。

mod media;
mod state;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use wonton_protocol::{
    AvatarMedia, AvatarMediaProblem, BotActivity, BotNameProblem, BotProfileUpdate,
    BotProfileUpdated, StateAvatars, check_bot_name,
};

pub use self::state::ProfileRevisions;
pub(super) use self::state::{EditWindow, ProfileState};
use super::{PluginId, PluginManager};
use crate::matrix::{Matrix, MatrixError, Session};

pub struct Profile {
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub avatars: StateAvatars,
}

pub struct UploadedAvatar {
    pub filename: String,
    pub data: Vec<u8>,
    pub poster: Option<(String, Vec<u8>)>,
}

pub enum MediaChange {
    Set(UploadedAvatar),
    Clear,
}

#[derive(Default)]
pub struct LocalUpdate {
    pub name: Option<String>,
    pub avatar: Option<MediaChange>,
    pub states: BTreeMap<BotActivity, MediaChange>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("not_found: 插件没有安装或没有 Bot")]
    NotFound,
    #[error("invalid_input: {0}")]
    InvalidInput(String),
    #[error("invalid_input: Bot 名字不符合规则（{0:?}）")]
    Name(BotNameProblem),
    #[error("media_rejected: 头像文件不符合规则（{0:?}）")]
    Media(AvatarMediaProblem),
    #[error("matrix_unavailable: {0}")]
    MatrixUnavailable(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("rate_limited: 每小时最多成功改 10 次资料，还要等 {retry_after_secs} 秒")]
    RateLimited { retry_after_secs: u64 },
}

impl ProfileError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::InvalidInput(_) | Self::Name(_) => "invalid_input",
            Self::Media(_) => "media_rejected",
            Self::MatrixUnavailable(_) => "matrix_unavailable",
            Self::Storage(_) => "storage",
            Self::RateLimited { .. } => "rate_limited",
        }
    }
}

impl From<MatrixError> for ProfileError {
    fn from(error: MatrixError) -> Self {
        Self::MatrixUnavailable(error.to_string())
    }
}

struct Changes {
    name: Option<String>,
    avatar: Option<Option<String>>,
    states: BTreeMap<BotActivity, Option<AvatarMedia>>,
}

impl Changes {
    async fn apply(
        self,
        session: &Session<'_>,
        current: &mut Profile,
        written: &mut bool,
    ) -> Result<(), MatrixError> {
        if let Some(name) = self.name {
            session.set_display_name(&name).await?;
            *written = true;
            current.name = Some(name);
        }
        if let Some(avatar) = self.avatar {
            session.set_avatar_url(avatar.as_deref()).await?;
            *written = true;
            current.avatar = avatar;
        }
        if !self.states.is_empty() {
            let avatars = &mut current.avatars;
            for (state, media) in self.states {
                match media {
                    Some(media) => {
                        avatars.states.insert(state.key().to_owned(), media);
                    }
                    None => {
                        avatars.states.remove(state.key());
                    }
                }
            }
            session.set_avatars(avatars).await?;
            *written = true;
        }
        Ok(())
    }
}

impl PluginManager {
    /// 活动接口从这里取得同一份版本表，不另建缓存。
    pub fn profile_revisions(&self) -> Arc<ProfileRevisions> {
        self.profiles.revisions.clone()
    }

    pub(super) fn profile_lock(&self, id: &PluginId) -> Arc<tokio::sync::Mutex<EditWindow>> {
        self.profiles.lock(id)
    }

    fn profile_bot(&self, id: &PluginId) -> Result<(&Matrix, String), ProfileError> {
        let has_bot = self
            .plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| {
                listed
                    .manifest
                    .permissions
                    .contains(&super::Permission::MatrixBot)
            });
        if !has_bot {
            return Err(ProfileError::NotFound);
        }
        let bots = self
            .store
            .bots()
            .load(id)
            .map_err(|e| ProfileError::Storage(e.to_string()))?;
        // 资料按 Bot 的接口在别的任务里；现在默认 Bot 没有（或插件没记录）就报没找到。
        let record = bots
            .bots
            .get(super::bot_services::DEFAULT_BOT)
            .or_else(|| bots.bots.values().next())
            .ok_or(ProfileError::NotFound)?;
        let user = record.account.user_id.clone();
        let matrix = self
            .matrix
            .as_deref()
            .ok_or_else(|| ProfileError::MatrixUnavailable("核心没有接入 Matrix".to_owned()))?;
        Ok((matrix, user))
    }

    fn profile_name(&self, name: Option<String>) -> Result<Option<String>, ProfileError> {
        let Some(name) = name else {
            return Ok(None);
        };
        let name = name.trim();
        check_bot_name(name).map_err(ProfileError::Name)?;
        let folded = name.to_lowercase();
        let owner = self.cfg.profile.owner().display_name;
        if [
            "wonton",
            "云吞",
            "雲吞",
            owner.trim().to_lowercase().as_str(),
        ]
        .contains(&folded.as_str())
        {
            return Err(ProfileError::Name(BotNameProblem::Reserved));
        }
        Ok(Some(name.to_owned()))
    }

    pub async fn bot_profile(&self, id: &PluginId) -> Result<Profile, ProfileError> {
        let lock = self.profile_lock(id);
        let _editing = lock.lock().await;
        let (matrix, user) = self.profile_bot(id)?;
        Ok(matrix.as_bot(&user, read).await?)
    }

    pub async fn update_bot_profile(
        &self,
        id: &PluginId,
        update: BotProfileUpdate,
    ) -> Result<BotProfileUpdated, ProfileError> {
        let lock = self.profile_lock(id);
        let _editing = lock.lock().await;
        let (matrix, user) = self.profile_bot(id)?;
        ensure_change(
            update.name.is_some(),
            update.avatar.is_some(),
            !update.states.is_empty(),
        )?;
        let name = self.profile_name(update.name)?;
        let changes = media::remote(matrix, name, update.avatar, update.states).await?;
        let mut written = false;
        let result = matrix
            .as_bot(&user, async |session: &Session<'_>| {
                let mut current = read(session).await?;
                changes.apply(session, &mut current, &mut written).await
            })
            .await;
        let profile_rev = if written {
            let profile_rev = self.profiles.revisions.bump(id.as_str());
            self.activity.changed();
            profile_rev
        } else {
            self.profiles.revisions.get(id.as_str())
        };
        result?;
        Ok(BotProfileUpdated { profile_rev })
    }

    pub async fn set_bot_profile(
        &self,
        id: &PluginId,
        update: LocalUpdate,
    ) -> Result<Profile, ProfileError> {
        let lock = self.profile_lock(id);
        let mut editing = lock.lock().await;
        let (matrix, user) = self.profile_bot(id)?;
        ensure_change(
            update.name.is_some(),
            update.avatar.is_some(),
            !update.states.is_empty(),
        )?;
        editing.check(Instant::now())?;
        let name = self.profile_name(update.name)?;
        let ready = media::local(name, update.avatar, update.states)?;
        let mut written = false;
        let result = matrix
            .as_bot(&user, async |session: &Session<'_>| {
                let mut current = read(session).await?;
                ready
                    .upload(session)
                    .await?
                    .apply(session, &mut current, &mut written)
                    .await?;
                Ok(current)
            })
            .await;
        // Matrix profile fields are separate writes. A later error or failed logout
        // cannot hide a confirmed earlier mutation or let repeated partial edits evade quota.
        if written {
            editing.succeeded();
            self.profiles.revisions.bump(id.as_str());
            self.activity.changed();
        }
        Ok(result?)
    }
}

async fn read(session: &Session<'_>) -> Result<Profile, MatrixError> {
    Ok(Profile {
        name: session.display_name().await?,
        avatar: session.avatar_url().await?,
        avatars: session.avatars().await?.unwrap_or_default(),
    })
}

fn ensure_change(name: bool, avatar: bool, states: bool) -> Result<(), ProfileError> {
    if name || avatar || states {
        Ok(())
    } else {
        Err(ProfileError::InvalidInput("至少写一项资料".to_owned()))
    }
}

#[cfg(test)]
pub(crate) mod tests;
