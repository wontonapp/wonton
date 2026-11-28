//! 同一上传的操作排同一个队；一个上传的清理卡住不妨碍别的上传。

use std::time::Duration;

use super::*;

/// 还有人在排队时离开，后来的调用要和他们排同一个队，不能拿到一把新锁并发放同一个上传。
#[tokio::test]
async fn a_turn_someone_is_waiting_for_stays_in_the_table() {
    let f = Fixture::new();
    let uploads = &f.files.uploads;
    let id = new_id();

    let first = uploads.turn(&id).await;
    let mut waiting = Box::pin(uploads.turn(&id));
    assert!(futures_util::poll!(&mut waiting).is_pending());
    drop(first);
    let waiting = waiting.await;
    let mut later = Box::pin(uploads.turn(&id));
    assert!(
        futures_util::poll!(&mut later).is_pending(),
        "要和拿着锁的排同一个队"
    );
    drop(waiting);
    drop(later.await);
    assert!(uploads.finishing.lock().unwrap().is_empty());

    // 请求断了（排队的 future 被丢掉）也不留在表里。
    let held = uploads.turn(&id).await;
    let mut gave_up = Box::pin(uploads.turn(&id));
    assert!(futures_util::poll!(&mut gave_up).is_pending());
    drop(gave_up);
    drop(held);
    assert!(uploads.finishing.lock().unwrap().is_empty());
}

/// 作废要动 `transfers`，存储卡住时可能很久：别的上传问进度不用等，正在作废的如实说接不上了。
#[tokio::test]
async fn a_stuck_clean_up_holds_up_no_other_upload() {
    let f = Fixture::new();
    let stale = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    let hold = f.files.uploads.abandon_stalled.write().await;
    let files = f.files.clone();
    let sweeping = tokio::spawn(async move { files.sweep(now_secs() + 2 * STALE_SECS).await });
    // 只试着拿锁：作废时要是还拿着它，这里报超时，而不是一起卡住。
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f
            .files
            .uploads
            .abandoning
            .try_lock()
            .is_ok_and(|abandoning| !abandoning.is_empty())
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("作废的时候不拿着这把锁");
    let other = new_id();
    f.files.pending_for_test(&other, 5, "phone");
    let seen = tokio::time::timeout(Duration::from_secs(5), f.files.upload_seen(&other));
    assert!(seen.await.expect("别的上传不用等"));
    assert!(!f.files.upload_seen(&stale).await, "这个已经接不上了");
    drop(hold);
    sweeping.await.unwrap();
    assert_eq!(
        f.files.upload_state(&stale).await.unwrap(),
        UploadState::Unknown
    );
    assert!(f.files.uploads.abandoning.lock().unwrap().is_empty());
}
