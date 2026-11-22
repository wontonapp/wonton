//! 自己的头像（#662）：文件按文件头检查后传进 Matrix 媒体库，把 `mxc://` 写进自己的资料 `avatar_url`；
//! `avatar_url` 是标准 Matrix 用户资料字段，主人和家人走同一条路，自己和其他设备读到的都是服务器上的那一份。

use std::sync::Arc;

use wonton_avatar::inspect;
use wonton_matrix::{Error, MatrixSession};
use wonton_protocol::{
    AvatarImage, AvatarKind, AvatarMediaProblem, AvatarUpload, CoreError, Me, MediaHandle,
    MyAvatarProblem,
};

use super::Scope;

/// 换头像：先检查，再上传，最后把读回的 `mxc://` 写进资料。
pub(super) async fn set(scope: Arc<Scope>, upload: AvatarUpload) -> Result<Me, MyAvatarProblem> {
    let media =
        inspect(&upload.data).map_err(|problem| MyAvatarProblem::MediaRejected { problem })?;
    if media.duration_ms.is_some()
        || !matches!(
            media.mimetype.as_str(),
            "image/png" | "image/jpeg" | "image/webp"
        )
    {
        return Err(MyAvatarProblem::MediaRejected {
            problem: AvatarMediaProblem::Format,
        });
    }
    let session = session(&scope)?;
    let url = session
        .upload_media(&media.mimetype, upload.data)
        .await
        .map_err(problem)?;
    session
        .set_own_avatar_url(Some(&url))
        .await
        .map_err(problem)?
        .ok_or_else(|| MyAvatarProblem::Upstream {
            reason: "Matrix did not keep the avatar url".to_owned(),
        })?;
    me(scope).await
}

/// 清头像。Matrix 那边本来就是空的也照常成功。
pub(super) async fn clear(scope: Arc<Scope>) -> Result<Me, MyAvatarProblem> {
    let session = session(&scope)?;
    if session
        .set_own_avatar_url(None)
        .await
        .map_err(problem)?
        .is_some()
    {
        return Err(MyAvatarProblem::Upstream {
            reason: "Matrix did not clear the avatar url".to_owned(),
        });
    }
    me(scope).await
}

/// 把服务器上的资料拼在 `me.avatar` 上。聊天还没登录好时留空，头像只是资料的一部分，不影响整页。
pub(super) async fn attach(me: &mut Me, scope: &Scope) -> Result<(), CoreError> {
    let session = match scope.chat.current() {
        Ok(session) => session,
        Err(CoreError::NotReady { .. }) => return Ok(()),
        Err(error) => return Err(error),
    };
    let profile = session
        .fetch_user_profile(session.user_id())
        .await
        .map_err(core_problem)?;
    me.avatar = handle(profile.avatar_url.as_deref());
    Ok(())
}

async fn me(scope: Arc<Scope>) -> Result<Me, MyAvatarProblem> {
    let mut me = scope.plugins.me().await.map_err(me_problem)?;
    attach(&mut me, &scope).await.map_err(me_problem)?;
    Ok(me)
}

fn session(scope: &Scope) -> Result<Arc<MatrixSession>, MyAvatarProblem> {
    scope.chat.current().map_err(|error| match error {
        CoreError::NotReady { reason } => MyAvatarProblem::TransportUnavailable { reason },
        other => MyAvatarProblem::Upstream {
            reason: other.to_string(),
        },
    })
}

fn handle(url: Option<&str>) -> Option<AvatarImage> {
    let token = wonton_matrix::media_token(url?).ok()?;
    Some(AvatarImage {
        media: MediaHandle { token },
        kind: AvatarKind::Still,
        width: None,
        height: None,
        poster: None,
    })
}

fn problem(error: Error) -> MyAvatarProblem {
    match error {
        Error::Transport(reason) => MyAvatarProblem::TransportUnavailable { reason },
        Error::NotReady(reason) => MyAvatarProblem::Upstream { reason },
        Error::Unauthorized { errcode, message } | Error::Forbidden { errcode, message } => {
            MyAvatarProblem::Upstream {
                reason: format!("{errcode}: {message}"),
            }
        }
        Error::Upstream(reason) | Error::InvalidInput(reason) => {
            MyAvatarProblem::Upstream { reason }
        }
    }
}

fn core_problem(error: Error) -> CoreError {
    match error {
        Error::Transport(reason) => CoreError::TransportUnavailable { reason },
        Error::NotReady(reason) => CoreError::NotReady { reason },
        Error::Unauthorized { errcode, message } => CoreError::Unauthorized {
            reason: format!("{errcode}: {message}"),
        },
        Error::Forbidden { errcode, message } => CoreError::Forbidden {
            reason: format!("{errcode}: {message}"),
        },
        Error::Upstream(reason) | Error::InvalidInput(reason) => CoreError::Upstream { reason },
    }
}

fn me_problem(error: CoreError) -> MyAvatarProblem {
    match error {
        CoreError::TransportUnavailable { reason } | CoreError::NotReady { reason } => {
            MyAvatarProblem::TransportUnavailable { reason }
        }
        other => MyAvatarProblem::Upstream {
            reason: other.to_string(),
        },
    }
}
