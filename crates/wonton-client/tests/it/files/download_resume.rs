//! 断了的下载接着下：带上半截文件的长度和 ETag，文件变了从头下。

use super::*;

/// 接着下时带上半截文件的长度和 ETag。
#[tokio::test]
async fn a_download_continues_after_the_app_restarts() {
    let mut world = World::start().await;
    let bytes = source_bytes(4 * MIB);
    let file = world.core.lock().serve("a.mov", &bytes, "v1");
    world.core.lock().hold_download_after = Some(MIB);
    let destination = world.downloaded(&file, "a.mov");
    let part = world.downloaded(&file, "a.mov.part");
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile {
                id: file,
                room: "!bot:home".into(),
            },
        })
        .unwrap();
    // 等核心发的都写完再停：写到一半就读长度的话，停下之前可能又写了一些。
    world
        .wait_until(|| part.metadata().is_ok_and(|meta| meta.len() == MIB as u64))
        .await;
    assert!(!destination.exists());

    world.stop_phone();
    world.start_phone().await;
    world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;
    assert_eq!(std::fs::read(&destination).unwrap(), bytes);
    let core = world.core.lock();
    let (range, if_range) = core.ranges.last().unwrap();
    assert_eq!(range.as_deref(), Some(format!("bytes={MIB}-").as_str()));
    assert_eq!(if_range.as_deref(), Some("\"v1\""));
}

#[tokio::test]
async fn a_file_that_changed_while_paused_is_downloaded_from_the_start() {
    let mut world = World::start().await;
    let file = world
        .core
        .lock()
        .serve("a.mov", &source_bytes(4 * MIB), "v1");
    world.core.lock().hold_download_after = Some(MIB);
    let destination = world.downloaded(&file, "a.mov");
    let part = world.downloaded(&file, "a.mov.part");
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile {
                id: file.clone(),
                room: "!bot:home".into(),
            },
        })
        .unwrap();
    world
        .wait_until(|| part.metadata().is_ok_and(|meta| meta.len() > 0))
        .await;
    world.stop_phone();
    let changed = source_bytes(2 * MIB + 7);
    world
        .core
        .lock()
        .files
        .insert(file.to_string(), Served::new("a.mov", &changed, "v2"));

    world.start_phone().await;
    world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;
    assert_eq!(std::fs::read(&destination).unwrap(), changed);
}
