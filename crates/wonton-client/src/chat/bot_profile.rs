use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_matrix::{Error, MatrixSession};
use wonton_protocol::{
    AvatarChange, AvatarMediaProblem, AvatarUpload, BotActivity, BotNameProblem, BotProfileFailure,
    BotProfileProblem, BotProfileUpdate, BotProfileUpdated, CoreError, MAX_AVATAR_BYTES,
    check_bot_name,
};

use super::bots::Bots;
use super::{invalid, lock};
use crate::plugins::api::{CoreApiError, READ_TIMEOUT};

enum Edit {
    Update(BotProfileUpdate),
    Upload(Option<BotActivity>, AvatarUpload),
}

impl super::ChatLink {
    pub(crate) fn set_bot_name(&self, id: String, name: String) -> Result<(), CoreError> {
        self.bots.set_name(id, name)
    }

    pub(crate) fn set_bot_avatar(
        &self,
        id: String,
        state: Option<BotActivity>,
        upload: AvatarUpload,
    ) -> Result<(), CoreError> {
        self.bots.set_avatar(id, state, upload)
    }

    pub(crate) fn clear_bot_avatar(
        &self,
        id: String,
        state: Option<BotActivity>,
    ) -> Result<(), CoreError> {
        self.bots.clear_avatar(id, state)
    }

    pub(crate) fn dismiss_bot_profile_problem(&self, id: &str) {
        self.bots.dismiss_problem(id);
    }
}

impl Bots {
    pub fn set_name(self: &Arc<Self>, id: String, name: String) -> Result<(), CoreError> {
        let name = name.trim().to_owned();
        if let Err(problem) = check_bot_name(&name) {
            let detail = format!("Invalid Bot name: {problem:?}");
            self.problem(
                &id,
                BotProfileFailure::InvalidName { problem },
                detail.clone(),
            );
            return Err(invalid(&detail));
        }
        self.save(
            id,
            Edit::Update(BotProfileUpdate {
                name: Some(name),
                ..Default::default()
            }),
        )
    }

    pub fn set_avatar(
        self: &Arc<Self>,
        id: String,
        state: Option<BotActivity>,
        upload: AvatarUpload,
    ) -> Result<(), CoreError> {
        if upload.data.len() as u64 > MAX_AVATAR_BYTES
            || upload
                .poster
                .as_ref()
                .is_some_and(|poster| poster.len() as u64 > MAX_AVATAR_BYTES)
        {
            self.problem(
                &id,
                BotProfileFailure::MediaRejected {
                    problem: AvatarMediaProblem::TooLarge,
                },
                "Avatar or poster exceeds the avatar byte limit".to_owned(),
            );
            return Err(invalid("Avatar or poster exceeds the avatar byte limit"));
        }
        self.save(id, Edit::Upload(state, upload))
    }

    pub fn clear_avatar(
        self: &Arc<Self>,
        id: String,
        state: Option<BotActivity>,
    ) -> Result<(), CoreError> {
        self.save(id, Edit::Update(avatar_update(state, AvatarChange::Clear)))
    }

    pub fn dismiss_problem(&self, id: &str) {
        self.view.send_modify(|view| {
            if let Some(bot) = view.bots.iter_mut().find(|bot| bot.bot.plugin_id == id) {
                bot.problem = None;
            }
        });
    }

    fn problem(&self, id: &str, reason: BotProfileFailure, detail: String) {
        self.view.send_modify(|view| {
            if let Some(bot) = view.bots.iter_mut().find(|bot| bot.bot.plugin_id == id) {
                bot.problem = Some(BotProfileProblem { reason, detail });
            }
        });
    }

    fn save(self: &Arc<Self>, id: String, edit: Edit) -> Result<(), CoreError> {
        let Some(session) = self.session.borrow().clone() else {
            self.problem(
                &id,
                BotProfileFailure::NotReady,
                "Matrix is not signed in".to_owned(),
            );
            return Err(CoreError::NotReady {
                reason: "Matrix is not signed in".to_owned(),
            });
        };
        let (user, identity) = lock(&self.local)
            .records
            .get(&id)
            .map(|record| (record.user_id.clone(), record.identity))
            .ok_or_else(|| CoreError::NotReady {
                reason: "The plugin has no installed Bot".to_owned(),
            })?;
        let api = self.plugins.api();
        let mut busy = false;
        self.view.send_modify(|view| {
            if let Some(bot) = view.bots.iter_mut().find(|bot| bot.bot.plugin_id == id) {
                busy = bot.saving;
                if !busy {
                    bot.saving = true;
                    bot.problem = None;
                }
            }
        });
        if busy {
            return Err(CoreError::NotReady {
                reason: "Bot profile is already being saved".to_owned(),
            });
        }
        let bots = self.clone();
        let epoch = self.epoch.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let result = match api {
                Ok(api) => bots.submit(&api, &id, session, edit, epoch).await,
                Err(_) => Err(BotProfileProblem {
                    reason: BotProfileFailure::TransportUnavailable,
                    detail: "Cannot reach the home server".to_owned(),
                }),
            };
            if bots.epoch.load(Ordering::SeqCst) != epoch {
                return;
            }
            let local = lock(&bots.local);
            if !local
                .records
                .get(&id)
                .is_some_and(|record| record.user_id == user && record.identity == identity)
            {
                return;
            }
            let success = result.is_ok();
            bots.view.send_modify(|view| {
                if let Some(bot) = view.bots.iter_mut().find(|bot| bot.bot.plugin_id == id) {
                    bot.saving = false;
                    bot.problem = result.err();
                }
            });
            drop(local);
            if success {
                bots.refresh(&id);
            }
        });
        Ok(())
    }

    async fn submit(
        &self,
        api: &crate::plugins::api::CoreApi,
        id: &str,
        session: Arc<MatrixSession>,
        edit: Edit,
        epoch: u64,
    ) -> Result<(), BotProfileProblem> {
        let update = match edit {
            Edit::Update(update) => update,
            Edit::Upload(state, upload) => {
                let url = session
                    .upload_media(&upload.mime_type, upload.data)
                    .await
                    .map_err(matrix_problem)?;
                let poster = match upload.poster {
                    Some(poster) => Some(
                        session
                            .upload_media("image/jpeg", poster)
                            .await
                            .map_err(matrix_problem)?,
                    ),
                    None => None,
                };
                avatar_update(state, AvatarChange::Set { url, poster })
            }
        };
        if self.epoch.load(Ordering::SeqCst) != epoch
            || !self
                .session
                .borrow()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &session))
        {
            return Err(BotProfileProblem {
                reason: BotProfileFailure::NotReady,
                detail: "Matrix session changed while uploading the avatar".to_owned(),
            });
        }
        let _: BotProfileUpdated = api
            .put_detailed(&["bots", id, "profile"], &update, READ_TIMEOUT)
            .await
            .map_err(profile_problem)?;
        Ok(())
    }
}

fn avatar_update(state: Option<BotActivity>, change: AvatarChange) -> BotProfileUpdate {
    let mut update = BotProfileUpdate::default();
    match state {
        Some(state) => {
            update.states.insert(state, change);
        }
        None => update.avatar = Some(change),
    }
    update
}

fn matrix_problem(error: Error) -> BotProfileProblem {
    let (reason, detail) = match error {
        Error::Transport(detail) => (BotProfileFailure::TransportUnavailable, detail),
        Error::NotReady(detail) => (BotProfileFailure::NotReady, detail),
        Error::Unauthorized { errcode, message } | Error::Forbidden { errcode, message } => {
            (BotProfileFailure::Upstream, format!("{errcode}: {message}"))
        }
        Error::Upstream(detail) | Error::InvalidInput(detail) => {
            (BotProfileFailure::Upstream, detail)
        }
    };
    BotProfileProblem {
        reason,
        detail: format!("Matrix avatar upload failed: {detail}"),
    }
}

fn profile_problem(error: CoreApiError) -> BotProfileProblem {
    match error {
        CoreApiError::Transport(_) => BotProfileProblem {
            reason: BotProfileFailure::TransportUnavailable,
            detail: "Cannot reach the home server".to_owned(),
        },
        CoreApiError::Protocol(detail) => BotProfileProblem {
            reason: BotProfileFailure::Upstream,
            detail,
        },
        CoreApiError::Http {
            status,
            kind,
            message,
            problem,
        } => {
            let reason = match (status, kind.as_deref()) {
                (404, _) => BotProfileFailure::NotReady,
                // 只有主人的接口对家人的设备回 403 `permission_required`（ADR-0045）：不是网络问题，按权限缺失如实显示。
                (403, Some("permission_required")) => BotProfileFailure::PermissionRequired,
                (403 | 502, None) => BotProfileFailure::TransportUnavailable,
                (400, Some("invalid_input")) => BotProfileFailure::InvalidName {
                    problem: problem
                        .and_then(|problem| serde_json::from_value(problem.into()).ok())
                        .unwrap_or(BotNameProblem::Other),
                },
                (400, Some("media_rejected")) => BotProfileFailure::MediaRejected {
                    problem: problem
                        .and_then(|problem| serde_json::from_value(problem.into()).ok())
                        .unwrap_or(AvatarMediaProblem::Other),
                },
                _ => BotProfileFailure::Upstream,
            };
            BotProfileProblem {
                reason,
                detail: format!("Bot profile request returned HTTP {status}: {message}"),
            }
        }
    }
}
