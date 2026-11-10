//! 传完但链接没发出去的上传 24 小时后清掉；授权给房间、发到设备以后不再过期（ADR-0020 的“生命周期”）。

use std::fs;

use wonton_protocol::FileId;

use super::super::new_id;
use super::super::record::{Accessor, Lookup};
use super::super::share_fs::TRANSFERS;
use super::super::test_support::{Fixture, any_langs};
use super::{UPLOAD_TTL_SECS, UploadState};
use crate::plugin::manager::now_secs;

/// 传完放好的一条上传，返回编号和公开的名字。
async fn placed(f: &Fixture, id: &FileId) -> std::path::PathBuf {
    f.files
        .create_upload(
            super::NewUpload {
                id,
                name: Some("a.txt"),
                mime_type: None,
                size: Some(5),
                device: "phone",
            },
            &any_langs(),
        )
        .await
        .unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    f.files.finish_upload(id, &any_langs()).await.unwrap();
    f.shares_dir.join(TRANSFERS).join("a.txt")
}

/// 把记录改成 `at` 过期。
fn expires_at(f: &Fixture, id: &FileId, at: u64) {
    let Lookup::Found(mut record) = f.files.registry.get(id, now_secs()) else {
        panic!("没有 {id}");
    };
    record.expires_at = Some(at);
    f.files.registry.save(record).unwrap();
}

fn expire(f: &Fixture, id: &FileId, now: u64) {
    expires_at(f, id, now.saturating_sub(1));
}

/// 传完没发链接：到期后文件和记录都没了，审计写明是到期删的。
#[tokio::test]
async fn an_upload_nobody_sent_expires_with_its_file_and_record() {
    let f = Fixture::new();
    let id = new_id();
    let public = placed(&f, &id).await;
    let now = now_secs();
    expire(&f, &id, now);

    f.files.sweep(now).await;

    assert!(!public.exists());
    assert!(matches!(f.files.registry.get(&id, now), Lookup::Unknown));
    assert_eq!(
        f.files.upload_state(&id).await.unwrap(),
        UploadState::Unknown
    );
    let audited = f.audit_lines();
    let last = audited.last().unwrap();
    assert_eq!(last["event"], "file_delete");
    assert_eq!(last["device"], "expiry");
    assert_eq!(last["outcome"], "expired");
}

/// 授权给房间（房间里没有 Bot 也算）或者发到设备，链接就算发出去了，不再过期。
#[tokio::test]
async fn sending_the_link_stops_the_expiry() {
    for send_to_devices in [false, true] {
        let f = Fixture::new();
        let id = new_id();
        let public = placed(&f, &id).await;
        let now = now_secs();
        // 到期前一秒把链接发出去。
        let soon = now.saturating_add(1);
        expires_at(&f, &id, soon);
        if send_to_devices {
            f.files.send_to_devices(&id, &any_langs()).await.unwrap();
        } else {
            f.files
                .grant_room(&id, "!room", &any_langs())
                .await
                .unwrap();
        }

        f.files.sweep(soon.saturating_add(UPLOAD_TTL_SECS)).await;

        assert!(public.exists(), "发过链接的不删");
        let Lookup::Found(record) = f.files.registry.get(&id, now) else {
            panic!("发过链接的记录要留着");
        };
        assert_eq!(record.expires_at, None, "不再过期");
    }
}

/// 到期时名字上已经是主人换掉的文件：文件留着，记录也留着，只是不再过期，下个小时不再白删。
#[tokio::test]
async fn a_file_the_owner_replaced_at_expiry_keeps_its_record() {
    let f = Fixture::new();
    let id = new_id();
    let public = placed(&f, &id).await;
    let now = now_secs();
    fs::write(&public, "the owner's own file").unwrap();
    expire(&f, &id, now);

    f.files.sweep(now).await;

    assert_eq!(fs::read_to_string(&public).unwrap(), "the owner's own file");
    let Lookup::Found(record) = f.files.registry.get(&id, now) else {
        panic!("记录要留着");
    };
    assert_eq!(record.expires_at, None);
    assert!(record.access.contains(&Accessor::Owner));
}

/// 文件已经没了：记录跟着删，不留一张打不开的卡片。
#[tokio::test]
async fn a_file_already_gone_still_loses_its_record() {
    let f = Fixture::new();
    let id = new_id();
    let public = placed(&f, &id).await;
    let now = now_secs();
    fs::remove_file(&public).unwrap();
    expire(&f, &id, now);

    f.files.sweep(now).await;

    assert!(matches!(f.files.registry.get(&id, now), Lookup::Unknown));
}

/// 清理开始之后才授权出去的：再查一次是正常记录，不删。
#[tokio::test]
async fn an_upload_granted_after_the_scan_is_kept() {
    let f = Fixture::new();
    let id = new_id();
    let public = placed(&f, &id).await;
    let now = now_secs();
    expire(&f, &id, now);
    f.files.registry.keep(&id).unwrap();

    let outcome = f.files.delete_expired_now(&id, now).unwrap();

    assert_eq!(outcome, "kept");
    assert!(public.exists());
}

/// 回收槽坏了删不掉：记录和文件都留着，两次清理都还认得出来，别把记录清成一张打不开的卡片。
#[tokio::test]
async fn a_delete_that_keeps_failing_keeps_the_record_and_file() {
    let f = Fixture::new();
    let id = new_id();
    let public = placed(&f, &id).await;
    let now = now_secs();
    expire(&f, &id, now);
    let transfers = f.shares_dir.join(TRANSFERS);
    fs::remove_dir_all(transfers.join(".wonton-trash")).unwrap();

    f.files.sweep(now).await;
    // 第二次跑过 24 小时：`purge_expired` 会清过期的记录，上传的记录要跳过，否则文件成了孤儿。
    let later = now.saturating_add(2 * UPLOAD_TTL_SECS);
    f.files.sweep(later).await;
    assert!(public.exists());
    assert!(matches!(
        f.files.registry.get(&id, later),
        Lookup::Expired(_)
    ));
}
