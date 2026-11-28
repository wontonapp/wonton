use wonton_matrix::UserProfile;
use wonton_protocol::{
    AVATARS_FIELD, AvatarImage, AvatarKind, AvatarMedia, BotActivity, BotView, MediaHandle,
    StateAvatar, StateAvatars,
};

pub(super) fn handle(url: &str) -> Option<MediaHandle> {
    match wonton_matrix::media_token(url) {
        Ok(token) => Some(MediaHandle { token }),
        Err(error) => {
            tracing::warn!(%error, "Invalid Bot avatar MXC");
            None
        }
    }
}

fn image(media: &AvatarMedia) -> Option<AvatarImage> {
    let kind = if media.mimetype.starts_with("video/") {
        AvatarKind::Video
    } else if media.mimetype == "image/gif"
        || (media.mimetype == "image/webp" && media.duration_ms.is_some())
    {
        AvatarKind::Animated
    } else {
        AvatarKind::Still
    };
    Some(AvatarImage {
        media: handle(&media.url)?,
        kind,
        width: Some(media.width),
        height: Some(media.height),
        poster: media.poster.as_deref().and_then(handle),
    })
}

/// 现在该播的状态头像。壳按它显示，没有时用静态头像 `avatar`，再没有用插件图标。
#[uniffi::export]
pub fn bot_current_avatar(bot: BotView) -> Option<AvatarImage> {
    bot.current_state_avatar().cloned()
}

pub(super) fn profile_view(
    mut profile: UserProfile,
    fallback: &str,
) -> (String, Option<AvatarImage>, Vec<StateAvatar>) {
    let avatar = profile
        .avatar_url
        .as_deref()
        .and_then(handle)
        .map(|media| AvatarImage {
            media,
            kind: AvatarKind::Still,
            width: None,
            height: None,
            poster: None,
        });
    let states = profile.fields.remove(AVATARS_FIELD).and_then(|value| {
        serde_json::from_value::<StateAvatars>(value)
            .inspect_err(|error| tracing::warn!(%error, "Invalid Bot state avatars"))
            .ok()
    });
    let state_avatars = states.map_or_else(Vec::new, |avatars| {
        BotActivity::ALL
            .into_iter()
            .filter_map(|state| {
                avatars
                    .states
                    .get(state.key())
                    .and_then(image)
                    .map(|image| StateAvatar { state, image })
            })
            .collect()
    });
    (
        profile.display_name.unwrap_or_else(|| fallback.to_owned()),
        avatar,
        state_avatars,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::BotRef;

    fn view(activity: Option<BotActivity>, states: &[(&str, BotActivity)]) -> BotView {
        BotView {
            shared: false,
            danger: None,
            chat: Default::default(),
            availability: Default::default(),
            user_id: "@bot:home".into(),
            bot: BotRef {
                plugin_id: "app.bot".into(),
                bot: "default".into(),
            },
            web: None,
            web_page: None,
            name: "Bot".into(),
            avatar: None,
            state_avatars: states
                .iter()
                .map(|(url, state)| StateAvatar {
                    state: *state,
                    image: AvatarImage {
                        media: MediaHandle {
                            token: (*url).to_owned(),
                        },
                        kind: AvatarKind::Still,
                        width: None,
                        height: None,
                        poster: None,
                    },
                })
                .collect(),
            activity,
            saving: false,
            problem: None,
            cloud: None,
            face: None,
            custom_media: false,
        }
    }

    #[test]
    fn the_shell_plays_the_state_avatar_and_falls_back_to_idle() {
        let both = view(
            Some(BotActivity::Working),
            &[
                ("idle", BotActivity::Idle),
                ("working", BotActivity::Working),
            ],
        );
        assert_eq!(bot_current_avatar(both).unwrap().media.token, "working");
        // 没有这个状态的素材时用待机的。
        let idle_only = view(Some(BotActivity::Working), &[("idle", BotActivity::Idle)]);
        assert_eq!(bot_current_avatar(idle_only).unwrap().media.token, "idle");
    }

    #[test]
    fn uncertain_or_unsourced_states_leave_the_choice_to_the_static_avatar() {
        assert_eq!(
            bot_current_avatar(view(None, &[("idle", BotActivity::Idle)]))
                .map(|image| image.media.token),
            None
        );
        assert_eq!(
            bot_current_avatar(view(Some(BotActivity::Working), &[])),
            None
        );
    }
}
