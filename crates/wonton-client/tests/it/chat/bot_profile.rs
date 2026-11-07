use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;
use wonton_client::{
    Action, AvatarKind, AvatarMediaProblem, AvatarUpload, BotActivity, BotActivityFacts,
    BotNameProblem, BotProfileFailure, BotView, CoreError, MediaHandle, TimelineContent,
};
use wonton_matrix::MatrixSession;
use wonton_protocol::{AVATARS_FIELD, PushMethod};

use super::PNG;
use super::bot_profile_support::{set_field, typing};
use super::support::{Home, Phone, Timeline};
use crate::plugins::rig::running;

const PLUGIN: &str = "app.bot";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，真实 Tuwunel 和模拟核心验证 Bot 资料与状态"]
async fn bot_profiles_actions_activity_and_cached_video_through_the_tunnel() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    let bot = Arc::new(home.bot("profilebot", "Original Bot").await);
    home.bot_api.target(&home.matrix.url, bot.clone());
    let avatar = bot.upload_media("image/png", PNG.to_vec()).await.unwrap();
    let video = bot
        .upload_media("video/mp4", b"avatar video".to_vec())
        .await
        .unwrap();
    set_field(
        &home.matrix.url,
        &bot,
        "avatar_url",
        json!({"avatar_url":avatar}),
    )
    .await;
    set_field(
        &home.matrix.url,
        &bot,
        AVATARS_FIELD,
        json!({AVATARS_FIELD:{"states":{
            "idle":{"url":video,"mimetype":"video/mp4","w":480,"h":480,"size":12},
            "working":{"url":avatar,"mimetype":"image/webp","w":2,"h":2,"size":70,"duration_ms":10},
            "waiting":{"url":avatar,"mimetype":"image/gif","w":2,"h":2,"size":70},
            "offline":{"url":avatar,"mimetype":"image/webp","w":2,"h":2,"size":70},
            "future":{"url":avatar,"mimetype":"image/png","w":2,"h":2,"size":70}
        }}}),
    )
    .await;
    let mut installed = running(PLUGIN);
    installed.title = "Plugin fallback".to_owned();
    installed.bot_user_id = Some(bot.user_id().to_string());
    home.set_installed_plugins(vec![installed]);
    let mut facts = facts(&bot);
    home.bot_api.facts(facts.clone());
    phone.connect(&home);
    phone.signed_in().await;
    let view = wait_bot(&phone, |bot| {
        bot.name == "Original Bot" && bot.state_avatars.len() == 4
    })
    .await;
    assert_eq!(view.avatar.as_ref().unwrap().kind, AvatarKind::Still);
    assert_eq!(
        phone.load_media(view.avatar.unwrap().media).await.unwrap(),
        PNG
    );
    assert_eq!(
        view.state_avatars
            .iter()
            .map(|avatar| avatar.image.kind)
            .collect::<Vec<_>>(),
        [
            AvatarKind::Video,
            AvatarKind::Animated,
            AvatarKind::Animated,
            AvatarKind::Still
        ]
    );
    cache_video(&phone, &home, view.state_avatars[0].image.media.clone()).await;
    actions(&phone, &home).await;
    profile_refreshes(&phone, &home, &bot, &avatar, &mut facts).await;
    let room = direct_room(&phone, &bot).await;
    status_transitions(&phone, &home, &bot, room.as_str(), &mut facts).await;
    let mut timeline = Timeline::open(&phone, room.as_str()).await;
    bot.set_display_name("Renamed by Bot").await.unwrap();
    timeline
        .until("结构化资料变化", |item| {
            matches!(&item.content, TimelineContent::ProfileChange { user_id, name, .. }
            if user_id == bot.user_id().as_str() && name == "Renamed by Bot")
            .then_some(())
        })
        .await;
    wait_bot(&phone, |bot| bot.name == "Renamed by Bot").await;
    home.bot_api.broken.send_replace(true);
    wait_bot(&phone, |bot| bot.activity.is_none()).await;
    // 后台暂停轮询的前提是收新消息走 UnifiedPush（ADR-0003）：保持连接是默认方式，退到后台也照常轮询。
    phone
        .dispatch(Action::SetPushMethod {
            method: PushMethod::UnifiedPush,
        })
        .unwrap();
    phone.dispatch(Action::AppBackgrounded).unwrap();
    let polls = home.bot_api.polls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(3_200)).await;
    assert_eq!(home.bot_api.polls.load(Ordering::SeqCst), polls);
    drop(timeline);
    home.stop_tunnel().await;
    drop(phone);
    bot.stop_sync().await;
}

fn facts(bot: &MatrixSession) -> BotActivityFacts {
    BotActivityFacts {
        plugin_id: PLUGIN.to_owned(),
        user_id: bot.user_id().to_string(),
        running: true,
        model_requests: 0,
        tool_calls: 0,
        waiting: 0,
        profile_rev: 0,
    }
}

async fn profile_refreshes(
    phone: &Phone,
    home: &Home,
    bot: &MatrixSession,
    avatar: &str,
    facts: &mut BotActivityFacts,
) {
    set_field(
        &home.matrix.url,
        bot,
        AVATARS_FIELD,
        json!({AVATARS_FIELD:{"states":{
            "idle":{"url":avatar,"mimetype":"image/png","w":2,"h":2,"size":70}
        }}}),
    )
    .await;
    facts.profile_rev = 50;
    home.bot_api.facts(facts.clone());
    wait_bot(phone, |bot| {
        bot.state_avatars.len() == 1 && bot.state_avatars[0].image.kind == AvatarKind::Still
    })
    .await;
    phone.dispatch(Action::AppBackgrounded).unwrap();
    set_field(
        &home.matrix.url,
        bot,
        AVATARS_FIELD,
        json!({AVATARS_FIELD:{"states":"invalid"}}),
    )
    .await;
    phone.dispatch(Action::AppForegrounded).unwrap();
    wait_bot(phone, |bot| bot.state_avatars.is_empty()).await;
    set_field(
        &home.matrix.url,
        bot,
        AVATARS_FIELD,
        json!({AVATARS_FIELD:{"states":{
            "working":{"url":avatar,"mimetype":"image/gif","w":2,"h":2,"size":70}
        }}}),
    )
    .await;
    let mut installed = home_plugin(bot);
    installed.revision = "changed".to_owned();
    home.set_installed_plugins(vec![installed]);
    phone.dispatch(Action::RefreshPlugins).unwrap();
    wait_bot(phone, |bot| {
        bot.state_avatars.len() == 1 && bot.state_avatars[0].state == BotActivity::Working
    })
    .await;
}

fn home_plugin(bot: &MatrixSession) -> wonton_client::InstalledPlugin {
    let mut installed = running(PLUGIN);
    installed.title = "Plugin fallback".to_owned();
    installed.bot_user_id = Some(bot.user_id().to_string());
    installed
}

async fn direct_room(phone: &Phone, bot: &MatrixSession) -> wonton_matrix::OwnedRoomId {
    let room = bot
        .create_direct_chat(
            &super::support::OWNER
                .parse::<wonton_matrix::OwnedUserId>()
                .unwrap(),
            false,
        )
        .await
        .unwrap();
    phone
        .wait_chat("收到 Bot 邀请", |view| {
            view.rooms
                .iter()
                .find(|row| {
                    row.room_id == room.as_str()
                        && row.membership == wonton_client::RoomMembership::Invited
                        && row.is_direct
                })
                .cloned()
        })
        .await;
    phone
        .dispatch(Action::JoinRoom {
            room_id: room.to_string(),
        })
        .unwrap();
    phone
        .wait_chat("加入 Bot 私聊", |view| {
            view.rooms
                .iter()
                .find(|row| {
                    row.room_id == room.as_str()
                        && row.membership == wonton_client::RoomMembership::Joined
                        && row.direct_user_id.as_deref() == Some(bot.user_id().as_str())
                })
                .cloned()
        })
        .await;
    room
}
async fn wait_bot(phone: &Phone, check: impl Fn(&BotView) -> bool) -> BotView {
    phone
        .wait_chat("Bot ViewModel", |view| {
            view.bots
                .iter()
                .find(|bot| bot.plugin_id == PLUGIN && check(bot))
                .cloned()
        })
        .await
}

async fn cache_video(phone: &Phone, home: &Home, media: MediaHandle) {
    let before = home.bot_api.downloads.load(Ordering::SeqCst);
    let results =
        futures_util::future::join_all((0..8).map(|_| phone.avatar_file(media.clone()))).await;
    let paths: Vec<_> = results.into_iter().map(Result::unwrap).collect();
    assert!(paths.iter().all(|path| path == &paths[0]));
    assert_eq!(std::fs::read(&paths[0]).unwrap(), b"avatar video");
    assert_eq!(home.bot_api.downloads.load(Ordering::SeqCst), before + 1);
    assert_eq!(phone.avatar_file(media).await.unwrap(), paths[0]);
    assert_eq!(home.bot_api.downloads.load(Ordering::SeqCst), before + 1);
}

async fn actions(phone: &Phone, home: &Home) {
    assert!(matches!(
        phone.dispatch(Action::SetBotName {
            plugin_id: PLUGIN.into(),
            name: " \n ".into()
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(
        wait_bot(phone, |bot| bot.problem.is_some())
            .await
            .problem
            .unwrap()
            .reason,
        BotProfileFailure::InvalidName {
            problem: BotNameProblem::Empty
        }
    );
    phone
        .dispatch(Action::SetBotName {
            plugin_id: PLUGIN.into(),
            name: "  Named by owner  ".into(),
        })
        .unwrap();
    assert!(phone.chat().bots[0].saving);
    wait_bot(phone, |bot| bot.name == "Named by owner" && !bot.saving).await;
    assert_eq!(
        home.bot_api.requests.lock().unwrap()[0],
        json!({"name":"Named by owner"})
    );
    failures(phone, home).await;
    let uploads = home.bot_api.events.lock().unwrap().len();
    assert!(matches!(
        phone.dispatch(Action::SetBotAvatar {
            plugin_id: PLUGIN.into(),
            state: None,
            upload: AvatarUpload {
                mime_type: "image/png".into(),
                data: vec![0; wonton_protocol::MAX_AVATAR_BYTES as usize + 1],
                poster: None
            },
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(home.bot_api.events.lock().unwrap().len(), uploads);
    phone
        .dispatch(Action::DismissBotProfileProblem {
            plugin_id: PLUGIN.into(),
        })
        .unwrap();
    home.bot_api.events.lock().unwrap().clear();
    phone
        .dispatch(Action::SetBotAvatar {
            plugin_id: PLUGIN.into(),
            state: Some(BotActivity::Waiting),
            upload: AvatarUpload {
                mime_type: "video/mp4".into(),
                data: b"new video".to_vec(),
                poster: Some(PNG.to_vec()),
            },
        })
        .unwrap();
    wait_bot(phone, |bot| !bot.saving && bot.problem.is_none()).await;
    assert_eq!(
        *home.bot_api.events.lock().unwrap(),
        ["upload", "upload", "profile"]
    );
    let last = home
        .bot_api
        .requests
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(last["states"]["waiting"]["action"], "set");
    assert!(
        last["states"]["waiting"]["url"]
            .as_str()
            .unwrap()
            .starts_with("mxc://")
    );
    assert!(
        last["states"]["waiting"]["poster"]
            .as_str()
            .unwrap()
            .starts_with("mxc://")
    );
    phone
        .dispatch(Action::ClearBotAvatar {
            plugin_id: PLUGIN.into(),
            state: None,
        })
        .unwrap();
    wait_bot(phone, |bot| !bot.saving && bot.avatar.is_none()).await;
    assert_eq!(
        *home.bot_api.requests.lock().unwrap().last().unwrap(),
        json!({"avatar":{"action":"clear"}})
    );
}

async fn failures(phone: &Phone, home: &Home) {
    for (status, kind, problem, expected) in [
        (404, "not_found", "", BotProfileFailure::NotReady),
        (
            400,
            "invalid_input",
            "reserved",
            BotProfileFailure::InvalidName {
                problem: BotNameProblem::Reserved,
            },
        ),
        (
            400,
            "invalid_input",
            "new_problem",
            BotProfileFailure::InvalidName {
                problem: BotNameProblem::Other,
            },
        ),
        (
            400,
            "media_rejected",
            "too_long",
            BotProfileFailure::MediaRejected {
                problem: AvatarMediaProblem::TooLong,
            },
        ),
        (
            400,
            "media_rejected",
            "new_problem",
            BotProfileFailure::MediaRejected {
                problem: AvatarMediaProblem::Other,
            },
        ),
        (500, "upstream", "", BotProfileFailure::Upstream),
    ] {
        *home.bot_api.next_reply.lock().unwrap() = Some((
            status,
            json!({"error":{
                "kind":kind,"message":"Diagnostic information","problem":problem
            }}),
        ));
        phone
            .dispatch(Action::SetBotName {
                plugin_id: PLUGIN.into(),
                name: "Another name".into(),
            })
            .unwrap();
        let view = wait_bot(phone, |bot| !bot.saving && bot.problem.is_some()).await;
        assert_eq!(view.problem.unwrap().reason, expected);
        phone
            .dispatch(Action::DismissBotProfileProblem {
                plugin_id: PLUGIN.into(),
            })
            .unwrap();
        assert_eq!(phone.chat().bots[0].problem, None);
    }
    *home.bot_api.next_reply.lock().unwrap() = Some((502, serde_json::Value::Null));
    phone
        .dispatch(Action::SetBotName {
            plugin_id: PLUGIN.into(),
            name: "Disconnected".into(),
        })
        .unwrap();
    let bot = wait_bot(phone, |bot| !bot.saving && bot.problem.is_some()).await;
    assert_eq!(
        bot.problem.unwrap().reason,
        BotProfileFailure::TransportUnavailable
    );
    phone
        .dispatch(Action::DismissBotProfileProblem {
            plugin_id: PLUGIN.into(),
        })
        .unwrap();
}

async fn status_transitions(
    phone: &Phone,
    home: &Home,
    bot: &MatrixSession,
    room: &str,
    facts: &mut BotActivityFacts,
) {
    wait_bot(phone, |bot| bot.activity == Some(BotActivity::Idle)).await;
    facts.model_requests = 1;
    home.bot_api.facts(facts.clone());
    wait_bot(phone, |bot| bot.activity == Some(BotActivity::Working)).await;
    facts.waiting = 1;
    home.bot_api.facts(facts.clone());
    wait_bot(phone, |bot| bot.activity == Some(BotActivity::Waiting)).await;
    facts.waiting = 0;
    facts.model_requests = 0;
    let quiet = tokio::time::Instant::now();
    home.bot_api.facts(facts.clone());
    wait_bot(phone, |bot| bot.activity == Some(BotActivity::Idle)).await;
    assert!(quiet.elapsed() >= Duration::from_secs(2));
    typing(&home.matrix.url, bot, room, true).await;
    wait_bot(phone, |bot| bot.activity == Some(BotActivity::Working)).await;
    typing(&home.matrix.url, bot, room, false).await;
    wait_bot(phone, |bot| bot.activity == Some(BotActivity::Idle)).await;
    facts.running = false;
    home.bot_api.facts(facts.clone());
    wait_bot(phone, |bot| bot.activity == Some(BotActivity::Offline)).await;
    phone.dispatch(Action::AppBackgrounded).unwrap();
    assert_eq!(phone.chat().bots[0].activity, None);
    phone.dispatch(Action::AppForegrounded).unwrap();
    wait_bot(phone, |bot| bot.activity == Some(BotActivity::Offline)).await;
}
