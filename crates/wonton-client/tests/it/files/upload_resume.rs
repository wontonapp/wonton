//! 断了的上传接着传（ADR-0020、ADR-0042）：重启、切服务器、掉线后从断点续；源变了或核心忘了就重来。

use super::*;

/// App 被杀：重启后问核心传到哪了，从那里接着传，不重传也不漏。
#[tokio::test]
async fn an_upload_continues_after_the_app_restarts() {
    let mut world = World::start().await;
    let bytes = source_bytes(4 * MIB);
    let source = world.write("a.mp4", &bytes);
    world.core.lock().hold_patch_after = Some(MIB);
    world
        .phone()
        .dispatch(upload(path_source(&source), "a.mp4"))
        .unwrap();
    world.wait_core(|core| core.uploaded_bytes() >= MIB).await;

    world.restart_phone().await;
    let transfer = world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;

    let core = world.core.lock();
    assert_eq!(core.patches, [0, MIB as u64], "第二次从断点开始");
    assert_eq!(core.uploads[transfer.file.unwrap().as_str()].bytes, bytes);
}

/// 切到另一台服务器时传到一半（ADR-0042）：离开时停在原处，切回来接着传，不在别的服务器上继续。
#[tokio::test]
async fn an_upload_left_by_switching_servers_continues_when_switching_back() {
    let world = World::start().await;
    let bytes = source_bytes(4 * MIB);
    let source = world.write("a.mp4", &bytes);
    world.core.lock().hold_patch_after = Some(MIB);
    world
        .phone()
        .dispatch(upload(path_source(&source), "a.mp4"))
        .unwrap();
    world.wait_core(|core| core.uploaded_bytes() >= MIB).await;

    world
        .phone()
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AddServer,
        })
        .unwrap();
    world
        .wait_until(|| world.phone().transfers().active.is_empty())
        .await;
    world
        .phone()
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::CancelAddServer,
        })
        .unwrap();
    // 切回来就接着传：作用域是新的，家里的连接的样子（已连上）直接看得见，不用等一轮重试。
    let transfer = tokio::time::timeout(Duration::from_secs(10), async {
        world
            .wait_transfer(|t| t.state == TransferState::Done)
            .await
    })
    .await
    .expect("切回来 10 秒内就该接着传");

    let core = world.core.lock();
    assert_eq!(core.patches, [0, MIB as u64], "切回来从断点开始");
    assert_eq!(core.uploads[transfer.file.unwrap().as_str()].bytes, bytes);
}

#[tokio::test]
async fn an_upload_continues_after_the_connection_drops() {
    let mut world = World::start().await;
    let bytes = source_bytes(4 * MIB);
    let source = world.write("a.mp4", &bytes);
    world.core.lock().hold_patch_after = Some(MIB);
    world
        .phone()
        .dispatch(upload(path_source(&source), "a.mp4"))
        .unwrap();
    world.wait_core(|core| core.uploaded_bytes() >= MIB).await;

    world.drop_connections().await;
    world
        .phone()
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
    let transfer = world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;
    assert_eq!(
        world.core.lock().uploads[transfer.file.unwrap().as_str()].bytes,
        bytes
    );
}

#[tokio::test]
async fn a_source_that_changed_while_paused_is_not_stitched_together() {
    let mut world = World::start().await;
    let source = world.write("a.mp4", &source_bytes(4 * MIB));
    world.core.lock().hold_patch_after = Some(MIB);
    world
        .phone()
        .dispatch(upload(path_source(&source), "a.mp4"))
        .unwrap();
    world.wait_core(|core| core.uploaded_bytes() >= MIB).await;
    world.stop_phone();
    std::fs::write(&source, b"changed").unwrap();

    world.start_phone().await;
    let transfer = world
        .wait_transfer(|t| matches!(t.state, TransferState::Failed { .. }))
        .await;
    failed_with(&transfer.state, FileFailure::LocalSourceUnavailable);
}

#[tokio::test]
async fn an_upload_the_core_forgot_has_to_start_over() {
    let mut world = World::start().await;
    let source = world.write("a.mp4", &source_bytes(4 * MIB));
    world.core.lock().hold_patch_after = Some(MIB);
    world
        .phone()
        .dispatch(upload(path_source(&source), "a.mp4"))
        .unwrap();
    world.wait_core(|core| core.uploaded_bytes() >= MIB).await;
    world.stop_phone();
    world.core.lock().uploads.clear();

    world.start_phone().await;
    let transfer = world
        .wait_transfer(|t| matches!(t.state, TransferState::Failed { .. }))
        .await;
    failed_with(&transfer.state, FileFailure::FileUnknown);
}
