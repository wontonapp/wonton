use std::collections::BTreeMap;

use wonton_protocol::{AvatarChange, AvatarMedia, AvatarMediaProblem, BotActivity};

use super::{Changes, MediaChange, ProfileError, UploadedAvatar};
use crate::bot_media::inspect;
use crate::matrix::{Matrix, MatrixError, Session};

fn checked(data: &[u8], still: bool) -> Result<AvatarMedia, ProfileError> {
    let media = inspect(data).map_err(ProfileError::Media)?;
    if still
        && (media.duration_ms.is_some()
            || !matches!(
                media.mimetype.as_str(),
                "image/png" | "image/jpeg" | "image/webp"
            ))
    {
        return Err(ProfileError::Media(AvatarMediaProblem::Format));
    }
    Ok(media)
}

async fn download(matrix: &Matrix, url: &str) -> Result<Vec<u8>, ProfileError> {
    matrix
        .download_media(url)
        .await
        .map_err(|error| match error {
            MatrixError::Invalid(_) => ProfileError::Media(AvatarMediaProblem::Format),
            other => other.into(),
        })
}

async fn remote_change(
    matrix: &Matrix,
    change: AvatarChange,
    still: bool,
) -> Result<Option<AvatarMedia>, ProfileError> {
    match change {
        AvatarChange::Clear => Ok(None),
        AvatarChange::Set { url, poster } => {
            let mut media = checked(&download(matrix, &url).await?, still)?;
            if let Some(poster) = &poster {
                checked(&download(matrix, poster).await?, true)?;
            }
            media.url = url;
            media.poster = poster;
            Ok(Some(media))
        }
    }
}

pub(super) async fn remote(
    matrix: &Matrix,
    name: Option<String>,
    avatar: Option<AvatarChange>,
    states: BTreeMap<BotActivity, AvatarChange>,
) -> Result<Changes, ProfileError> {
    let avatar = match avatar {
        Some(change) => Some(remote_change(matrix, change, true).await?.map(|m| m.url)),
        None => None,
    };
    let mut checked_states = BTreeMap::new();
    for (state, change) in states {
        checked_states.insert(state, remote_change(matrix, change, false).await?);
    }
    Ok(Changes {
        name,
        avatar,
        states: checked_states,
    })
}

struct LocalMedia {
    input: UploadedAvatar,
    media: AvatarMedia,
    poster_type: Option<String>,
}

impl LocalMedia {
    fn check(input: UploadedAvatar, still: bool) -> Result<Self, ProfileError> {
        let media = checked(&input.data, still)?;
        let poster_type = input
            .poster
            .as_ref()
            .map(|(_, bytes)| checked(bytes, true).map(|m| m.mimetype))
            .transpose()?;
        Ok(Self {
            input,
            media,
            poster_type,
        })
    }

    async fn upload(self, session: &Session<'_>) -> Result<AvatarMedia, MatrixError> {
        let mut media = self.media;
        media.url = session
            .upload_media(&media.mimetype, &self.input.filename, self.input.data)
            .await?;
        if let Some((filename, bytes)) = self.input.poster {
            let mimetype = self.poster_type.expect("首帧已在上传前检查");
            media.poster = Some(session.upload_media(&mimetype, &filename, bytes).await?);
        }
        Ok(media)
    }
}

pub(super) struct LocalChanges {
    name: Option<String>,
    avatar: Option<Option<LocalMedia>>,
    states: BTreeMap<BotActivity, Option<LocalMedia>>,
}

fn local_change(change: MediaChange, still: bool) -> Result<Option<LocalMedia>, ProfileError> {
    match change {
        MediaChange::Clear => Ok(None),
        MediaChange::Set(input) => LocalMedia::check(input, still).map(Some),
    }
}

pub(super) fn local(
    name: Option<String>,
    avatar: Option<MediaChange>,
    states: BTreeMap<BotActivity, MediaChange>,
) -> Result<LocalChanges, ProfileError> {
    let avatar = avatar
        .map(|change| local_change(change, true))
        .transpose()?;
    let states = states
        .into_iter()
        .map(|(state, change)| local_change(change, false).map(|m| (state, m)))
        .collect::<Result<_, _>>()?;
    Ok(LocalChanges {
        name,
        avatar,
        states,
    })
}

impl LocalChanges {
    pub(super) async fn upload(self, session: &Session<'_>) -> Result<Changes, MatrixError> {
        let avatar = match self.avatar {
            Some(Some(media)) => Some(Some(media.upload(session).await?.url)),
            Some(None) => Some(None),
            None => None,
        };
        let mut states = BTreeMap::new();
        for (state, media) in self.states {
            let uploaded = match media {
                Some(media) => Some(media.upload(session).await?),
                None => None,
            };
            states.insert(state, uploaded);
        }
        Ok(Changes {
            name: self.name,
            avatar,
            states,
        })
    }
}
