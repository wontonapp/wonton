use std::sync::Arc;
use std::thread;

use tempfile::TempDir;

use wonton_protocol::media::view::QuotaEntry;
use wonton_protocol::media::{
    FailedItem, ItemState, MediaFailure, MediaFailureCode, MediaItemRef, MediaKind, MediaTask,
    PauseReason, PlaybackPlan, PlaybackSession, Progress, ProgressUnit, Queue, QueueMode,
    StopReason, TaskId, TaskKind, TaskState, TaskTarget, UploadItem,
};

use super::store::{DEFAULT_PROVIDER_UPLOAD_LIMIT, Ledger, MediaStoreError, SCHEMA_V1, Store};

mod migrations;
mod open;
mod queue;
mod quota;
mod scoping;
mod sessions;
mod tasks;

const ALICE: &str = "@alice:one.example.test";
const BOB: &str = "@bob:two.example.test";
const PHONE: &str = "phone-a";
const IMMICH: &str = "immich";
const JELLYFIN: &str = "jellyfin";

fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

fn item(id: &str, offset: u64) -> UploadItem {
    UploadItem {
        item_id: id.into(),
        path: format!("/photos/{id}.jpg"),
        size: 4096,
        checksum: Some(format!("sha256:{id}")),
        taken_at: Some(1_700_000_000_000),
        state: ItemState::Queued,
        upload_offset: offset,
    }
}

fn failure() -> MediaFailure {
    MediaFailure {
        code: MediaFailureCode::UpstreamError,
        message: "提供方 502".into(),
        retry_after_secs: Some(7),
    }
}

/// 一个各字段都不取默认值的上传任务：读回不一致时能直接看出是哪一列。
fn upload_task(id: u128, person: &str) -> MediaTask {
    MediaTask {
        id: TaskId::from_u128(id),
        kind: TaskKind::Upload,
        person: person.into(),
        device: PHONE.into(),
        target: TaskTarget::Upload(vec![item("one", 7), item("two", 0)]),
        state: TaskState::Paused(PauseReason::LowBattery),
        progress: Progress {
            done: 1,
            total: Some(2),
            unit: ProgressUnit::Items,
        },
        retry_count: 2,
        retry_of: Some(TaskId::from_u128(999)),
        next_attempt_at: Some(1_700_000_010),
        created_at: 1_700_000_000_000,
        partial_failures: vec![FailedItem {
            item_id: "two".into(),
            path: "/photos/two.jpg".into(),
            failure: failure(),
        }],
    }
}

fn media_item(id: &str) -> MediaItemRef {
    MediaItemRef {
        id: id.into(),
        kind: MediaKind::Photo,
        title: format!("照片 {id}"),
        artist: None,
        album: None,
        artwork_url: Some("https://provider.example.test/a.jpg".into()),
        duration_ms: Some(5_000),
        series: None,
        index: Some(3),
        played: true,
        starred: false,
        source: None,
    }
}

fn session(person: &str, id: u128) -> PlaybackSession {
    PlaybackSession {
        id: TaskId::from_u128(id),
        person: person.into(),
        device: PHONE.into(),
        item: media_item("track"),
        state: TaskState::Running,
        position_ms: 42_000,
        plan: Some(PlaybackPlan {
            session: "provider-session".into(),
            stream: wonton_protocol::media::ProviderStream::Direct(
                "https://provider.example.test/stream".into(),
            ),
            origin_stream: None,
            segments: vec![],
            audio: vec![],
            subtitles: vec![],
            seekable: true,
            duration_ms: 180_000,
            expires_at: 1_700_000_100_000,
        }),
        stop_reason: None,
    }
}

fn queue() -> Queue {
    Queue {
        revision: 1,
        entries: vec![media_item("a"), media_item("b")],
        cursor: Some(1),
        mode: QueueMode::Shuffle,
        sleep_at_ms: Some(120_000),
    }
}
