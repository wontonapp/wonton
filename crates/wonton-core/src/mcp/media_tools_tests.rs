//! 媒体工具（ADR-0113 第 5 节）：替谁办事、只看这个人的、改任务受封顶。

use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use wonton_protocol::media::{MediaTask, PlaybackSession, PlaybackSessionId, new_task_id};
use wonton_protocol::{AiLevel, BotKeeper};

use super::ai_level_tests::{caller, member, notes, role};
use super::result::FAILURE_META;
use super::test_setup::Setup;
use super::tests::{failed, text};
use super::{Caller, TurnSender};
use crate::device::Devices;
use crate::http::HttpClient;
use crate::media::Media;
use crate::media::test_support::FakeDirectory;

const ALICE: &str = "@alice:wonton.localhost";
const BOB: &str = "@bob:wonton.localhost";
const STATUS: &str = "wonton__media_status";
const SESSIONS: &str = "wonton__media_sessions";
const TASK: &str = "wonton__media_task";

fn task(person: &str, state: Value) -> MediaTask {
    serde_json::from_value(json!({
        "id": new_task_id(), "kind": "upload", "person": person, "device": "phone",
        "target": {"kind": "upload", "value": [
            {"item_id": "a", "path": "DCIM/a.jpg", "size": 1, "checksum": null, "taken_at": null,
             "state": {"state": "queued"}, "upload_offset": 0},
            {"item_id": "b", "path": "DCIM/b.jpg", "size": 1, "checksum": null, "taken_at": null,
             "state": {"state": "queued"}, "upload_offset": 0},
        ]},
        "state": state, "progress": {"done": 0, "total": 2, "unit": "items"},
        "retry_count": 0, "retry_of": null, "next_attempt_at": null, "created_at": 0,
        "partial_failures": [],
    }))
    .unwrap()
}

fn session(person: &str, device: &str) -> PlaybackSession {
    serde_json::from_value(json!({
        "id": PlaybackSessionId::new_v4(), "person": person, "device": device,
        "item": {"id": "t1", "kind": "audio", "title": "夜曲", "artist": null, "album": null,
                 "artwork_url": null, "duration_ms": 10000, "series": null, "index": null,
                 "played": false, "starred": false},
        "state": "running", "position_ms": 4000, "plan": null, "stop_reason": null,
    }))
    .unwrap()
}

/// 接上媒体服务；alice 是能动手的家人，bob 也在家里。
fn attach(s: &Setup) -> (Arc<Media>, Arc<Devices>) {
    let writes = role(s, "能动手", AiLevel::AdminApprovesDeletes, true);
    member(s, "alice", &writes);
    member(s, "bob", &writes);
    let (directory, _) = FakeDirectory::new(Vec::new());
    let media = Arc::new(
        Media::with_directory(
            &s.dir.path().join("media"),
            Arc::new(directory),
            HttpClient::new(),
        )
        .unwrap(),
    );
    let devices = Arc::new(Devices::open(s.dir.path().join("devices.json")).unwrap());
    s.bus.attach_media(media.clone(), devices.clone());
    (media, devices)
}

fn alice_bot(sender: Option<&str>) -> Caller {
    let mut bot = caller(
        "alice-bot",
        BotKeeper::User {
            username: "alice".into(),
        },
    );
    bot.sender = sender.map(|who| TurnSender::Local(who.into()));
    bot
}

fn shared_bot(sender: Option<&str>) -> Caller {
    let mut bot = caller("family", BotKeeper::Household);
    bot.sender = sender.map(|who| TurnSender::Local(who.into()));
    bot
}

async fn ok(s: &Setup, caller: &Caller, tool: &str, args: Value) -> Value {
    let result = s.call_as(caller, tool, args).await;
    assert!(!failed(&result), "{}", text(&result));
    serde_json::from_str(&text(&result)).unwrap()
}

async fn refused(s: &Setup, caller: &Caller, tool: &str, args: Value) -> String {
    let result: CallToolResult = s.call_as(caller, tool, args).await;
    assert!(failed(&result), "{}", text(&result));
    result.meta.unwrap().0[FAILURE_META]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn a_private_bot_sees_only_its_keepers_tasks_and_sessions() {
    let s = notes().await;
    let me = alice_bot(Some(ALICE));
    assert_eq!(
        refused(&s, &me, STATUS, json!({})).await,
        "plugin_unavailable"
    );
    let (media, _) = attach(&s);
    let mine = task(ALICE, json!("queued"));
    for task in [&mine, &task(BOB, json!("queued"))] {
        media.store.create_task(task).unwrap();
    }
    media
        .store
        .create_session(&session(ALICE, "phone"))
        .unwrap();
    media.store.create_session(&session(BOB, "phone")).unwrap();

    let status = ok(&s, &me, STATUS, json!({})).await;
    assert_eq!(status["person"], ALICE);
    let tasks = status["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], json!(mine.id));
    // 上传的条目只报条数，不把整张清单塞给 Agent。
    assert_eq!(tasks[0]["items"], 2);
    assert_eq!(status["sessions"].as_array().unwrap().len(), 1);
    let only_sessions = ok(&s, &me, STATUS, json!({"kind": "playback"})).await;
    assert_eq!(only_sessions["tasks"], json!([]));
    // 写自己能认，写别人不行；私人 Bot 不能替说话的别人查。
    ok(&s, &me, STATUS, json!({"person": "alice"})).await;
    assert_eq!(
        refused(&s, &me, STATUS, json!({"person": BOB})).await,
        "permission_denied"
    );
}

#[tokio::test]
async fn a_shared_bot_acts_for_the_speaker_and_refuses_when_it_cannot_tell() {
    let s = notes().await;
    let (media, _) = attach(&s);
    media
        .store
        .create_task(&task(BOB, json!("queued")))
        .unwrap();
    let status = ok(&s, &shared_bot(Some(BOB)), STATUS, json!({})).await;
    assert_eq!(status["person"], BOB);
    assert_eq!(status["tasks"].as_array().unwrap().len(), 1);
    for who in [
        None,
        Some("@carol:wonton.localhost"),
        Some("@bob:elsewhere.example.com"),
    ] {
        assert_eq!(
            refused(&s, &shared_bot(who), STATUS, json!({})).await,
            "permission_denied",
            "{who:?}"
        );
    }
    assert_eq!(
        refused(
            &s,
            &shared_bot(Some(BOB)),
            STATUS,
            json!({"person": "alice"})
        )
        .await,
        "permission_denied"
    );
}

#[tokio::test]
async fn sessions_list_the_persons_own_devices() {
    let s = notes().await;
    let (media, devices) = attach(&s);
    let phone = iroh::SecretKey::from_bytes(&[7; 32]).public();
    let owners = iroh::SecretKey::from_bytes(&[8; 32]).public();
    devices
        .add_if(
            phone,
            Some("alice 的手机".into()),
            Some("alice".into()),
            |_| true,
        )
        .unwrap();
    devices.add(owners, Some("主人的平板".into())).unwrap();
    let label = crate::files::device_label(&phone);
    media.store.create_session(&session(ALICE, &label)).unwrap();

    let listed = ok(&s, &alice_bot(Some(ALICE)), SESSIONS, json!({})).await;
    let listed = listed["devices"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["device"], label);
    assert_eq!(listed[0]["sessions"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_task_tool_changes_only_the_subjects_tasks_within_the_ai_cap() {
    let s = notes().await;
    let (media, _) = attach(&s);
    let me = alice_bot(Some(ALICE));
    let queued = task(ALICE, json!("queued"));
    let failed_task = task(
        ALICE,
        json!({"failed": {"code": "upstream_error", "message": "提供方 500"}}),
    );
    let bobs = task(BOB, json!("queued"));
    for task in [&queued, &failed_task, &bobs] {
        media.store.create_task(task).unwrap();
    }

    let paused = ok(&s, &me, TASK, json!({"task": queued.id, "op": "pause"})).await;
    assert_eq!(paused["state"], json!({"paused": "agent"}));
    let first = ok(
        &s,
        &me,
        TASK,
        json!({"task": failed_task.id, "op": "retry"}),
    )
    .await;
    assert_eq!(first["retry_of"], json!(failed_task.id));
    // 同一个任务重试两次回的是同一条，不会排出两条。
    let again = ok(
        &s,
        &me,
        TASK,
        json!({"task": failed_task.id, "op": "retry"}),
    )
    .await;
    assert_eq!(again["id"], first["id"]);
    // 别人的任务当没有。
    assert_eq!(
        refused(&s, &me, TASK, json!({"task": bobs.id, "op": "cancel"})).await,
        "media_gone"
    );
    // 说不出这轮替谁办事：按只读封顶，查照常、改任务被拒。
    let unsure = alice_bot(None);
    ok(&s, &unsure, STATUS, json!({})).await;
    assert_eq!(
        refused(
            &s,
            &unsure,
            TASK,
            json!({"task": queued.id, "op": "resume"})
        )
        .await,
        "permission_denied"
    );
}
