//! 上传：名字和类型原样送到核心，源由路径或壳打开，失败保留本机原因。

use super::*;

#[tokio::test]
async fn an_upload_reaches_the_core_with_its_name_and_type() {
    let world = World::start().await;
    let bytes = source_bytes(3 * MIB);
    let source = world.write("a.mp4", &bytes);
    world
        .phone()
        .dispatch(upload(path_source(&source), "假期.mp4"))
        .unwrap();
    let transfer = world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;

    assert_eq!(transfer.direction, TransferDirection::Upload);
    assert_eq!(transfer.name, "假期.mp4");
    assert_eq!(transfer.transferred_bytes, bytes.len() as u64);
    assert_eq!(transfer.total_bytes, Some(bytes.len() as u64));
    let file = transfer.file.expect("传完有编号");
    let core = world.core.lock();
    let upload = &core.uploads[file.as_str()];
    assert_eq!(upload.bytes, bytes);
    assert_eq!(upload.metadata["filename"], "假期.mp4");
    assert_eq!(upload.metadata["filetype"], "video/mp4");
}

#[tokio::test]
async fn an_empty_file_finishes_without_sending_bytes() {
    let world = World::start().await;
    let source = world.write("empty.txt", b"");
    world
        .phone()
        .dispatch(upload(path_source(&source), "empty.txt"))
        .unwrap();
    world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;
    assert!(world.core.lock().patches.is_empty());
}

#[tokio::test]
async fn a_full_disk_at_home_lands_on_the_transfer() {
    let world = World::start().await;
    let source = world.write("a.mp4", b"abc");
    world.core.lock().storage_full = true;
    world
        .phone()
        .dispatch(upload(path_source(&source), "a.mp4"))
        .unwrap();
    let transfer = world
        .wait_transfer(|t| matches!(t.state, TransferState::Failed { .. }))
        .await;
    failed_with(&transfer.state, FileFailure::StorageFull);
    assert_eq!(transfer.file, None, "核心还没发编号");
}

#[tokio::test]
async fn a_source_that_is_gone_is_a_local_failure() {
    let world = World::start().await;
    let missing = world.path("missing.mp4");
    world
        .phone()
        .dispatch(upload(path_source(&missing), "a.mp4"))
        .unwrap();
    let transfer = world
        .wait_transfer(|t| matches!(t.state, TransferState::Failed { .. }))
        .await;
    failed_with(&transfer.state, FileFailure::LocalSourceUnavailable);
}

struct Opener(Result<std::path::PathBuf, FileOpenError>);

impl FileOpener for Opener {
    fn open_source(&self, handle: String) -> Result<i32, FileOpenError> {
        assert_eq!(handle, "content://media/1", "原样交给壳");
        let path = self.0.clone()?;
        Ok(File::open(path).unwrap().into_raw_fd())
    }
}

#[tokio::test]
async fn opaque_sources_are_opened_by_the_shell() {
    let world = World::start().await;
    let source = world.write("a.mp4", b"from the shell");
    world.phone().set_file_opener(Arc::new(Opener(Ok(source))));
    let opaque = UploadSource::Opaque {
        handle: "content://media/1".to_owned(),
    };
    world.phone().dispatch(upload(opaque, "a.mp4")).unwrap();
    let transfer = world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;
    let core = world.core.lock();
    assert_eq!(
        core.uploads[transfer.file.unwrap().as_str()].bytes,
        b"from the shell"
    );
}

#[tokio::test]
async fn failures_opening_a_source_keep_their_local_reason() {
    let world = World::start().await;
    let opaque = || UploadSource::Opaque {
        handle: "content://media/1".to_owned(),
    };
    world
        .phone()
        .dispatch(upload(opaque(), "no-opener.mp4"))
        .unwrap();
    let transfer = world
        .wait_named("no-opener.mp4", |t| {
            matches!(t.state, TransferState::Failed { .. })
        })
        .await;
    failed_with(&transfer.state, FileFailure::LocalSourceUnavailable);

    let denied = FileOpenError::PermissionDenied {
        reason: "SecurityException".to_owned(),
    };
    world.phone().set_file_opener(Arc::new(Opener(Err(denied))));
    world
        .phone()
        .dispatch(upload(opaque(), "denied.mp4"))
        .unwrap();
    let transfer = world
        .wait_named("denied.mp4", |t| {
            matches!(t.state, TransferState::Failed { .. })
        })
        .await;
    failed_with(&transfer.state, FileFailure::LocalPermissionDenied);
}
