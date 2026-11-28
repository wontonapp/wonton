//! 命令通道走 HTTP 的行为（ADR-0113 第 5 节）：工具排命令等回执，设备长轮询取、回执；回执幂等；
//! 别的设备的命令取不到、回执不了。等不到回执的情形在 `media::commands` 的测试里；夹具在
//! [`super::media_tests`]。

use axum::http::StatusCode;
use hyper::Method;
use serde_json::{Value, json};
use wonton_protocol::media::api::{command_ack_path, commands_path};
use wonton_protocol::media::{MediaAction, TaskId};

use super::media_tests::{Fixture, fixture};
use crate::media::commands::{COMMAND_ACK, CommandOutcome};

fn start(task: u128) -> MediaAction {
    MediaAction::StartDownload {
        task: TaskId::from_u128(task),
    }
}

async fn poll(f: &Fixture, device: &str, after: u64) -> (StatusCode, Value) {
    let path = format!("{}?device={device}&after={after}", commands_path());
    f.json(Method::GET, &path, Value::Null).await
}

#[tokio::test(start_paused = true)]
async fn a_command_reaches_the_device_and_its_ack_reaches_the_tool() {
    let f = fixture().await;
    let device = f.device();
    let media = f.media.clone();
    let target = device.clone();
    let tool = tokio::spawn(async move {
        media
            .commands
            .deliver(&target, &start(1), COMMAND_ACK)
            .await
    });
    // 设备先挂上长轮询，命令一排进来就回。
    let (status, list) = poll(&f, &device, 0).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let command = &list["commands"][0];
    assert_eq!(command["seq"], 1);
    assert_eq!(command["body"]["kind"], "start_download");
    let id: TaskId = command["command_id"].as_str().unwrap().parse().unwrap();
    let ack = json!({"seq": 1, "ok": false,
                     "failure": {"code": "local_storage_full", "message": "phone is full"}});
    let (status, _) = f
        .json(Method::POST, &command_ack_path(id), ack.clone())
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // 重复回执没有副作用。
    let (status, _) = f.json(Method::POST, &command_ack_path(id), ack).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let outcome = tool.await.unwrap().unwrap();
    assert!(
        matches!(&outcome, CommandOutcome::Failed(failure) if failure.message == "phone is full"),
        "{outcome:?}"
    );
    // 回执过的不再给；没有新命令就挂满 25 秒回 204。
    let (status, _) = poll(&f, &device, 0).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test(start_paused = true)]
async fn a_device_cannot_read_or_ack_another_devices_commands() {
    let f = fixture().await;
    let (status, refused) = poll(&f, "someone-else", 0).await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("permission_denied"))
    );
    let other = f
        .media
        .store
        .push_command("someone-else", &start(1))
        .unwrap();
    let ack = json!({"seq": other.seq, "ok": true});
    let (status, refused) = f
        .json(Method::POST, &command_ack_path(other.command_id), ack)
        .await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("permission_denied"))
    );
    // 回执的 seq 要和命令对得上。
    let mine = f.media.store.push_command(&f.device(), &start(2)).unwrap();
    let ack = json!({"seq": mine.seq + 1, "ok": true});
    let (status, refused) = f
        .json(Method::POST, &command_ack_path(mine.command_id), ack)
        .await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("protocol"))
    );
}
