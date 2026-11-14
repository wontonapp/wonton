//! 文件的行为测试（ADR-0020）：只经由导出给原生壳的接口驱动，断言传输列表和核心收到的请求。隧道服务端和假的核心文件接口（含 tus）都跑在
//! 本机。真实的 tusd 和核心的行为见 `wonton-core` 的 `gateway::files_upload_tests`。

use std::fs::File;
use std::os::fd::IntoRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::files_support::{Served, World, id, read_opened, source_bytes};
use wonton_client::{
    Action, CoreError, FileCard, FileFailure, FileOpenError, FileOpener, FileOperation, FileSource,
    TransferDirection, TransferState, UploadSource,
};

pub(super) const MIB: usize = 1024 * 1024;

pub(super) fn path_source(path: &std::path::Path) -> UploadSource {
    UploadSource::Path {
        path: path.to_str().unwrap().to_owned(),
    }
}

pub(super) fn upload(source: UploadSource, name: &str) -> Action {
    Action::Files {
        action: wonton_protocol::FileAction::UploadFile {
            source,
            name: name.to_owned(),
            mime_type: Some("video/mp4".to_owned()),
            room: None,
        },
    }
}

/// 发进房间的上传：传完由核心授权并把链接发进这个房间。
pub(super) fn upload_with_room(source: UploadSource, name: &str) -> Action {
    Action::Files {
        action: wonton_protocol::FileAction::UploadFile {
            source,
            name: name.to_owned(),
            mime_type: Some("video/mp4".to_owned()),
            room: Some("!bot:home".to_owned()),
        },
    }
}

fn failed_with(state: &TransferState, expected: FileFailure) {
    match state {
        TransferState::Failed { reason, .. } => assert_eq!(*reason, expected, "{state:?}"),
        other => panic!("应该失败：{other:?}"),
    }
}

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
async fn a_file_name_cannot_write_outside_the_app_downloads() {
    let world = World::start().await;
    let bytes = source_bytes(1024);
    let long = format!("{}.pdf", "长".repeat(200));
    for name in [
        "a.pdf",
        "../../x.pdf",
        "..",
        "/etc/x",
        "a\\..\\b",
        "x\ny",
        ".hidden",
        long.as_str(),
    ] {
        let file = world.core.lock().serve(name, &bytes, "\"v1\"");
        world
            .phone()
            .dispatch(Action::Files {
                action: wonton_protocol::FileAction::DownloadFile { id: file.clone() },
            })
            .unwrap();
        let done = world
            .wait_transfer(|t| t.file.as_ref() == Some(&file) && t.state == TransferState::Done)
            .await;
        let destination = PathBuf::from(done.destination.unwrap());
        let expected = world.downloaded(&file, "x");
        assert_eq!(
            destination.parent().unwrap(),
            expected.parent().unwrap(),
            "{name}"
        );
        assert!(destination.file_name().unwrap().len() <= 240, "{name}");
        assert_eq!(std::fs::read(&destination).unwrap(), bytes, "{name}");
    }
    let view = world.phone().transfers();
    let long_one = view.downloaded.iter().find(|t| t.name == long).unwrap();
    assert!(long_one.destination.as_ref().unwrap().ends_with(".pdf"));
    let only_dots = view.downloaded.iter().find(|t| t.name == "..").unwrap();
    let only_dots_file = only_dots.file.as_ref().unwrap();
    assert!(
        only_dots
            .destination
            .as_ref()
            .unwrap()
            .ends_with(only_dots_file.as_str()),
        "清洗后什么都不剩时用编号"
    );
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

#[tokio::test]
async fn cancelling_an_upload_deletes_what_the_core_has_so_far() {
    let world = World::start().await;
    let source = world.write("a.mp4", &source_bytes(4 * MIB));
    world.core.lock().hold_patch_after = Some(MIB);
    world
        .phone()
        .dispatch(upload(path_source(&source), "a.mp4"))
        .unwrap();
    world.wait_core(|core| core.uploaded_bytes() >= MIB).await;
    let transfer = world.phone().transfers().active.pop().unwrap();

    assert!(matches!(
        world.phone().dispatch(Action::Files {
            action: wonton_protocol::FileAction::DismissTransfer {
                id: transfer.id.clone(),
                delete_local_file: false,
            }
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::CancelTransfer { id: transfer.id },
        })
        .unwrap();
    assert!(world.phone().transfers().active.is_empty());
    let file = transfer.file.unwrap().to_string();
    world
        .wait_core(|core| core.terminated.contains(&file))
        .await;
}

#[tokio::test]
async fn dismissing_a_finished_upload_leaves_the_file_alone() {
    let world = World::start().await;
    let source = world.write("a.mp4", b"abc");
    world
        .phone()
        .dispatch(upload(path_source(&source), "a.mp4"))
        .unwrap();
    let transfer = world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;

    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::CancelTransfer {
                id: transfer.id.clone(),
            },
        })
        .unwrap();
    assert_eq!(
        world.phone().transfers().uploaded.len(),
        1,
        "取消和传完撞上不算错，也不动它"
    );
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DismissTransfer {
                id: transfer.id,
                delete_local_file: false,
            },
        })
        .unwrap();
    assert!(world.phone().transfers().uploaded.is_empty());
    let core = world.core.lock();
    assert!(core.terminated.is_empty() && core.deleted.is_empty());
}

#[tokio::test]
async fn unknown_transfers_are_rejected() {
    let world = World::start().await;
    for action in [
        Action::Files {
            action: wonton_protocol::FileAction::CancelTransfer {
                id: "nope".to_owned(),
            },
        },
        Action::Files {
            action: wonton_protocol::FileAction::DismissTransfer {
                id: "nope".to_owned(),
                delete_local_file: false,
            },
        },
        Action::Files {
            action: wonton_protocol::FileAction::ResendFileLink {
                id: "nope".to_owned(),
            },
        },
    ] {
        assert!(matches!(
            world.phone().dispatch(action),
            Err(CoreError::InvalidInput { .. })
        ));
    }
}

#[tokio::test]
async fn a_download_appears_in_the_app_folder_only_when_complete() {
    let world = World::start().await;
    let bytes = source_bytes(3 * MIB);
    let file = world.core.lock().serve("a.mov", &bytes, "v1");
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile { id: file.clone() },
        })
        .unwrap();
    let transfer = world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;
    let destination = world.downloaded(&file, "a.mov");
    assert_eq!(transfer.direction, TransferDirection::Download);
    assert_eq!(transfer.name, "a.mov");
    assert_eq!(transfer.file, Some(file.clone()));
    assert_eq!(transfer.total_bytes, Some(bytes.len() as u64));
    assert_eq!(transfer.destination.as_deref(), destination.to_str());
    assert_eq!(std::fs::read(&destination).unwrap(), bytes);
    assert!(!world.downloaded(&file, "a.mov.part").exists());
}

#[tokio::test]
async fn downloading_again_replaces_the_finished_entry() {
    let world = World::start().await;
    let file = world
        .core
        .lock()
        .serve("a.pdf", &source_bytes(1024), "\"v1\"");
    let download = Action::Files {
        action: wonton_protocol::FileAction::DownloadFile { id: file },
    };
    world.phone().dispatch(download.clone()).unwrap();
    let first = world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;

    world.phone().dispatch(download).unwrap();
    let second = world
        .wait_transfer(|t| t.id != first.id && t.state == TransferState::Done)
        .await;

    let list = world.phone().transfers().downloaded;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, second.id);
}

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
            action: wonton_protocol::FileAction::DownloadFile { id: file },
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
            action: wonton_protocol::FileAction::DownloadFile { id: file.clone() },
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

#[tokio::test]
async fn download_failures_say_which_side_failed() {
    let world = World::start().await;
    let unknown = id(9);
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile {
                id: unknown.clone(),
            },
        })
        .unwrap();
    let transfer = world
        .wait_named(unknown.as_str(), |t| {
            matches!(t.state, TransferState::Failed { .. })
        })
        .await;
    failed_with(&transfer.state, FileFailure::FileUnknown);

    let file = world.core.lock().serve("a.mov", b"abc", "v1");
    // 该是目录的地方是个文件。
    let path = world.downloaded(&file, "a.mov");
    let folder = path.parent().unwrap();
    std::fs::create_dir_all(folder.parent().unwrap()).unwrap();
    std::fs::write(folder, b"x").unwrap();
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile { id: file },
        })
        .unwrap();
    let transfer = world
        .wait_named("a.mov", |t| matches!(t.state, TransferState::Failed { .. }))
        .await;
    failed_with(&transfer.state, FileFailure::LocalIo);
}

#[tokio::test]
async fn a_file_is_not_downloaded_twice_at_once() {
    let world = World::start().await;
    let file = world
        .core
        .lock()
        .serve("a.mov", &source_bytes(4 * MIB), "v1");
    world.core.lock().hold_download_after = Some(MIB);
    let download = Action::Files {
        action: wonton_protocol::FileAction::DownloadFile { id: file },
    };
    world.phone().dispatch(download.clone()).unwrap();
    assert!(matches!(
        world.phone().dispatch(download),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[tokio::test]
async fn cards_are_cached_until_the_file_is_deleted() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.jpg", b"abc", "v1");
    let card = world.phone().file_card(file.clone()).await;
    let FileCard::Ready { info } = card else {
        panic!("{card:?}");
    };
    assert_eq!(info.name, "a.jpg");
    assert_eq!(info.source, FileSource::Device);
    world.phone().file_card(file.clone()).await;
    assert_eq!(world.core.lock().info_requests, 1);

    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_client::FileAction::DeleteFile { id: file.clone() },
        })
        .unwrap();
    let deleted = file.to_string();
    world
        .wait_core(|core| core.deleted.contains(&deleted))
        .await;
    let card = world.phone().file_card(file).await;
    assert!(
        matches!(
            card,
            FileCard::Failed {
                reason: FileFailure::FileUnknown,
                ..
            }
        ),
        "{card:?}"
    );
}

#[tokio::test]
async fn failed_cards_are_asked_again() {
    let world = World::start().await;
    let file = id(1);
    world.phone().file_card(file.clone()).await;
    world
        .core
        .lock()
        .files
        .insert(file.to_string(), Served::new("a.jpg", b"a", "v1"));
    let card = world.phone().file_card(file).await;
    assert!(matches!(card, FileCard::Ready { .. }), "{card:?}");
}

#[tokio::test]
async fn thumbnails_return_bytes_or_the_reason() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.jpg", b"abc", "v1");
    world.core.lock().thumbnail(&file, Ok(b"jpeg".to_vec()));
    assert_eq!(
        world.phone().file_thumbnail(file.clone()).await.unwrap(),
        b"jpeg"
    );

    world
        .core
        .lock()
        .thumbnail(&file, Err(FileFailure::NotViewable));
    assert!(matches!(
        world.phone().file_thumbnail(file.clone()).await,
        Err(CoreError::InvalidInput { .. })
    ));
    world
        .core
        .lock()
        .thumbnail(&file, Err(FileFailure::NotShared));
    assert!(matches!(
        world.phone().file_thumbnail(file).await,
        Err(CoreError::Forbidden { .. })
    ));
}

#[tokio::test]
async fn operations_reach_the_core_and_failures_say_which_file() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.jpg", b"abc", "v1");
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::SendFileToDevices { id: file.clone() },
        })
        .unwrap();
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::GrantFileToRoom {
                id: file.clone(),
                room: "!bot:home".to_owned(),
            },
        })
        .unwrap();
    let expected = file.to_string();
    world
        .wait_core(|core| core.sent.contains(&expected) && core.grants.len() == 1)
        .await;

    world.core.lock().delete_fails_with = Some(FileFailure::FileChanged);
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_client::FileAction::DeleteFile { id: file.clone() },
        })
        .unwrap();
    let problem = world.wait_problem().await;
    assert_eq!(problem.operation, FileOperation::Delete);
    assert_eq!(problem.file, file);
    assert_eq!(problem.reason, FileFailure::FileChanged);

    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DismissFileProblem,
        })
        .unwrap();
    assert_eq!(world.phone().transfers().problem, None);
}

#[tokio::test]
async fn the_player_address_serves_ranges_only_with_the_token() {
    let world = World::start().await;
    let bytes = b"0123456789";
    let file = world.core.lock().serve("a.mov", bytes, "v1");
    let url = world.phone().file_stream_url(file).await.unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    let ranged = client
        .get(&url)
        .header("Range", "bytes=2-5")
        .send()
        .await
        .unwrap();
    assert_eq!(ranged.status(), 206);
    assert_eq!(ranged.headers()["content-range"], "bytes 2-5/10");
    assert_eq!(ranged.bytes().await.unwrap().as_ref(), b"2345");

    let head = client.head(&url).send().await.unwrap();
    assert_eq!(head.status(), 200);
    assert_eq!(head.headers()["content-length"], "10");
    assert_eq!(head.headers()["accept-ranges"], "bytes");
    assert_eq!(head.headers()["x-content-type-options"], "nosniff");

    let token = parsed.path_segments().unwrap().next().unwrap();
    let wrong = url.replace(token, &"0".repeat(token.len()));
    assert_eq!(client.get(&wrong).send().await.unwrap().status(), 404);
    let requests = world.core.lock().ranges.len();
    assert_eq!(requests, 2, "令牌不对的请求不进隧道");
}

#[tokio::test]
async fn without_a_home_nothing_pretends_to_work() {
    let dir = tempfile::tempdir().unwrap();
    let phone = wonton_client::ClientCore::with_network(
        dir.path().to_str().unwrap(),
        wonton_client::Network::DirectOnly,
    )
    .unwrap();
    let source = dir.path().join("a.mp4");
    assert!(matches!(
        phone.dispatch(upload(path_source(&source), "a.mp4")),
        Err(CoreError::NotReady { .. })
    ));
    let card = phone.file_card(id(1)).await;
    assert!(
        matches!(
            card,
            FileCard::Failed {
                reason: FileFailure::TransportUnavailable,
                ..
            }
        ),
        "{card:?}"
    );
    assert!(matches!(
        phone.file_thumbnail(id(1)).await,
        Err(CoreError::TransportUnavailable { .. })
    ));
    assert!(matches!(
        phone.file_stream_url(id(1)).await,
        Err(CoreError::TransportUnavailable { .. })
    ));
}

#[tokio::test]
async fn an_unreadable_list_does_not_stop_the_app() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("servers/1")).unwrap();
    std::fs::write(dir.path().join("servers/1/transfers.json"), b"not json").unwrap();
    let phone = wonton_client::ClientCore::with_network(
        dir.path().to_str().unwrap(),
        wonton_client::Network::DirectOnly,
    )
    .unwrap();
    assert_eq!(phone.transfers().all().count(), 0);
    assert_eq!(
        read_opened(&dir.path().join("servers/1/transfers.bad")),
        b"not json"
    );
}
