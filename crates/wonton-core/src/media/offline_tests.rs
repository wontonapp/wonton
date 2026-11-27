//! 离线占用这本账以设备上报为准（`PUT /offline`，ADR-0113“配额与清理”）：按设备分开记，配额看这个人
//! 所有设备之和。

use std::sync::Arc;

use wonton_protocol::media::{
    MediaFailureCode, MediaTask, Progress, ProgressUnit, TaskId, TaskKind, TaskState, TaskTarget,
};

use super::store::{Ledger, Store};
use super::task_service::{TaskService, TaskServiceError};

const ALICE: &str = "@alice:one.example.test";
const BOB: &str = "@bob:two.example.test";

#[test]
fn each_device_reports_only_its_own_share() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.report_offline(ALICE, "phone", 300).unwrap();
    let view = store.report_offline(ALICE, "tablet", 200).unwrap();
    assert_eq!(view.device_offline.used, 500);
    // 手机删了副本再报：只改手机那份，平板的不动；别人的账不受影响。
    store.report_offline(BOB, "phone", 999).unwrap();
    let view = store.report_offline(ALICE, "phone", 100).unwrap();
    assert_eq!(view.device_offline.used, 300);
    // 两次上报之间，下载写完往上加，下一次上报再对齐。
    store
        .add_quota(ALICE, Ledger::DeviceOffline, None, Some("tablet"), 50)
        .unwrap();
    assert_eq!(store.quota(ALICE).unwrap().device_offline.used, 350);
    // 平板被移除：它那份清零。
    let view = store.report_offline(ALICE, "tablet", 0).unwrap();
    assert_eq!(view.device_offline.used, 100);
    assert_eq!(store.quota(BOB).unwrap().device_offline.used, 999);
}

/// 续下按还差的字节看配额：上限 10、已占 4，已写 4 共 10 的还差 6，放得下；没写过的要整份 10，放不下。
#[test]
fn a_resumed_download_is_admitted_on_the_bytes_it_still_needs() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    let conn = rusqlite::Connection::open(dir.path().join("media.sqlite3")).unwrap();
    conn.execute(
        "INSERT INTO quota (person, ledger, provider, device, used, cap)
         VALUES (?, 'device_offline', '', 'phone', 4, 10)",
        rusqlite::params![ALICE],
    )
    .unwrap();
    let tasks = TaskService::new(store.clone());
    let resumed = download(1, 4);
    store.create_task(&resumed).unwrap();
    assert_eq!(
        tasks.begin(resumed.id, None).unwrap().state,
        TaskState::Running
    );
    let fresh = download(2, 0);
    store.create_task(&fresh).unwrap();
    let err = tasks.begin(fresh.id, None).unwrap_err();
    assert!(
        matches!(&err, TaskServiceError::Refused(f) if f.code == MediaFailureCode::LocalStorageFull),
        "{err}"
    );
}

fn download(id: u128, done: u64) -> MediaTask {
    let item = serde_json::from_value(serde_json::json!({
        "id": "v1", "kind": "video", "title": "v1", "artist": null, "album": null,
        "artwork_url": null, "duration_ms": null, "series": null, "index": null,
        "played": false, "starred": false, "source": null,
    }))
    .unwrap();
    MediaTask {
        id: TaskId::from_u128(id),
        kind: TaskKind::Download,
        person: ALICE.into(),
        device: "phone".into(),
        target: TaskTarget::Download(item),
        state: TaskState::Queued,
        progress: Progress {
            done,
            total: Some(10),
            unit: ProgressUnit::Bytes,
        },
        retry_count: 0,
        retry_of: None,
        next_attempt_at: None,
        created_at: 0,
        partial_failures: Vec::new(),
    }
}
