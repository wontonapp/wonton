//! 上传进度：报告推进到已发送、节流、晚读也不丢最后的状态。

use std::time::Duration;

use matrix_sdk::send_queue::RoomSendQueueUpdate;
use tokio::sync::broadcast::error::RecvError;

use super::{Chat, entry_id, send, send_big_image};
use crate::messaging::image_named;
use crate::support::eventually;
use wonton_matrix::{SendStatus, StoreConfig};

/// 发一张大图：本地回显带着上传进度（缩略图和原图合并计算，total 里有原图），最后变成已发送（#92）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_image_upload_reports_progress_until_it_is_sent() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    let updates = chat.alice_room.message_updates().await;
    // Tuwunel 的上限最低 10 MB，8 MB 能进队列又够上传走几步。
    const BIG: usize = 8 * 1024 * 1024;
    send_big_image(&chat.alice_room, BIG).await;
    let mut uploaded = 0u64;
    let mut total = 0u64;
    eventually("上传进度推进到已发送", updates, |messages| {
        let message = messages
            .iter()
            .find(|message| message.is_own && image_named(message, "big.png").is_some())?;
        match &message.status {
            SendStatus::Sending {
                progress: Some(progress),
            } => {
                assert!(progress.total >= progress.current, "进度倒挂：{progress:?}");
                uploaded = progress.current;
                total = progress.total;
                None
            }
            SendStatus::Sent => Some(()),
            SendStatus::Sending { progress: None } => None,
            SendStatus::Failed { error, .. } => panic!("发送失败：{error}"),
        }
    })
    .await;
    assert!(uploaded > 0, "已发送之前没见过上传进度");
    assert!(
        total >= BIG as u64,
        "总字节数里没有原图（原图 {BIG} 字节，total {total}）"
    );
}

/// 上传进度有节流：最多每 100 ms 一条，不是每 8 KB 一条（8 MB 是 1024 条）（#18）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn image_progress_updates_are_throttled() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    let (_, mut updates) = chat
        .alice_room
        .timeline()
        .await
        .room()
        .send_queue()
        .subscribe()
        .await
        .expect("订阅这个房间的发送更新");
    const BIG: usize = 8 * 1024 * 1024;
    send_big_image(&chat.alice_room, BIG).await;

    let mut txn = None;
    let mut first = None;
    let mut last = std::time::Instant::now();
    let mut progress = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(30), updates.recv()).await {
            // 这里只有这一张图，带进度的更新都归它；本地回显可能比第一条进度还晚到。
            Ok(Ok(RoomSendQueueUpdate::MediaUpload {
                related_to,
                progress: step,
                ..
            })) => {
                txn.get_or_insert(related_to);
                let now = std::time::Instant::now();
                first.get_or_insert(now);
                last = now;
                progress.push(step);
            }
            Ok(Ok(RoomSendQueueUpdate::SentEvent { transaction_id, .. }))
                if txn.as_ref() == Some(&transaction_id) =>
            {
                break;
            }
            Ok(Ok(_)) => {}
            Ok(Err(RecvError::Lagged(missed))) => panic!("读得晚了，挤掉 {missed} 条更新"),
            Ok(Err(RecvError::Closed)) | Err(_) => panic!("没等到已发送"),
        }
    }
    let span = last.duration_since(first.expect("没看到上传进度"));
    // 就算间隔够了，也有几条不等间隔的：缩略图和原图各自的第一条、各自传完的一条、发出去的那条。
    // 其余每条之间至少 100 ms，所以这以上只能多出 span/100 条。
    let allowed = span.as_millis() / 100 + 8;
    assert!(
        progress.len() as u128 <= allowed,
        "进度没有节流：{} 条 / {span:?}，最多允许 {allowed} 条（{BIG} 字节按 8 KB 是 1024 条）",
        progress.len()
    );
    let last_progress = progress.last().expect("没看到上传进度");
    assert_eq!(
        last_progress.current, last_progress.total,
        "进度没到 100% 就断了"
    );
}

/// 广播攒着没人读时，最后的状态也不会被后面的一串更新挤掉：先发图，再连发几十条文字，然后才读，
/// 一条更新都不该丢，图片最后是这个附件传完（100%）和那条事件发出去（#18）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn image_final_state_waits_for_a_late_reader() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    // 先订阅、先不读：一条文字两条更新（回显、发出），几十条就把通道填满。
    let (_, mut updates) = chat
        .alice_room
        .timeline()
        .await
        .room()
        .send_queue()
        .subscribe()
        .await
        .expect("订阅这个房间的发送更新");
    const BIG: usize = 8 * 1024 * 1024;
    send_big_image(&chat.alice_room, BIG).await;
    for index in 0..40 {
        send(&chat.alice_room, &format!("挤通道 {index}")).await;
    }
    // 时间线上看到图片已发送；这期间一条更新都不读。
    entry_id(&chat.alice_room, "图片发出", |message| {
        message.is_own
            && image_named(message, "big.png").is_some()
            && message.status == SendStatus::Sent
    })
    .await;

    // 攒到现在再一条条读：超过容量时第一条是 `Lagged`，丢掉的更新数就在里面。
    let mut txn = None;
    let mut progress = Vec::new();
    let mut last_sent = false;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), updates.recv()).await {
            // 这里只有这一张图，带进度的更新都归它；本地回显可能比第一条进度还晚到。
            Ok(Ok(RoomSendQueueUpdate::MediaUpload {
                related_to,
                progress: step,
                ..
            })) => {
                txn.get_or_insert(related_to);
                progress.push(step);
                last_sent = false;
            }
            Ok(Ok(RoomSendQueueUpdate::SentEvent { transaction_id, .. }))
                if txn.as_ref() == Some(&transaction_id) =>
            {
                last_sent = true;
            }
            Ok(Ok(_)) => {}
            Ok(Err(RecvError::Lagged(missed))) => {
                panic!(
                    "读得晚了，挤掉 {missed} 条更新（进度 {} 条）",
                    progress.len()
                );
            }
            Ok(Err(RecvError::Closed)) | Err(_) => break,
        }
    }

    let last_progress = progress.last().expect("没看到上传进度");
    assert_eq!(
        last_progress.current, last_progress.total,
        "进度没到 100% 就断了"
    );
    assert!(
        last_progress.total >= BIG,
        "总量里没有原图（原图 {BIG} 字节，total {}）",
        last_progress.total
    );
    assert!(last_sent, "最后一条不是已发送");
}
