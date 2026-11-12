//! 主人看到什么、能点什么（ADR-0020）：卡片上的按钮、链接没发出和重发、清理下载、播放失败的原因。只经由导出给原生壳的接口驱动。

use std::path::Path;
use std::time::Duration;

use crate::files_support::{World, id, source_bytes};
use wonton_client::{
    Action, CoreError, FileCard, FileCardAction, FileCardActions, FileFailure, FileInfo,
    FileProblem, TransferAction, TransferState, UploadSource,
};

const MIB: usize = 1024 * 1024;

async fn info(world: &World, file: wonton_client::FileId) -> FileInfo {
    let card = world.phone().file_card(file).await;
    let FileCard::Ready { info } = card else {
        panic!("{card:?}");
    };
    info
}

async fn actions_of(world: &World, file: &wonton_client::FileId) -> FileCardActions {
    let info = info(world, file.clone()).await;
    world.phone().file_card_actions(info)
}

async fn download(world: &World, file: &wonton_client::FileId) -> wonton_client::Transfer {
    world
        .phone()
        .dispatch(Action::DownloadFile { id: file.clone() })
        .unwrap();
    world
        .wait_transfer(|t| t.file.as_ref() == Some(file) && t.state == TransferState::Done)
        .await
}

fn open_at(path: &Path) -> FileCardAction {
    FileCardAction::Open {
        path: path.to_str().unwrap().to_owned(),
    }
}

/// 音视频默认边下边播，旁边能下载后用其他 App 打开。
#[tokio::test]
async fn audio_and_video_play_and_can_open_elsewhere() {
    let world = World::start().await;
    let file = world
        .core
        .lock()
        .serve("a.mp4", &source_bytes(4 * MIB), "v1");

    let before = actions_of(&world, &file).await;
    assert_eq!(before.tap, FileCardAction::Play);
    assert_eq!(before.open_elsewhere, Some(FileCardAction::Download));
    assert_eq!(before.download, None);

    world.core.lock().hold_download_after = Some(MIB);
    world
        .phone()
        .dispatch(Action::DownloadFile { id: file.clone() })
        .unwrap();
    let during = actions_of(&world, &file).await;
    assert_eq!(during.tap, FileCardAction::Play, "下载的时候也能放");
    assert_eq!(during.open_elsewhere, Some(FileCardAction::Wait));
    assert!(during.download.is_some());
}

#[tokio::test]
async fn other_files_download_and_then_open() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");

    let before = actions_of(&world, &file).await;
    assert_eq!(before.tap, FileCardAction::Download);
    assert_eq!(before.open_elsewhere, None);

    let done = download(&world, &file).await;
    let after = actions_of(&world, &file).await;
    assert_eq!(after.tap, open_at(&world.downloaded(&file, "a.pdf")));
    assert_eq!(after.download, Some(done));

    // 文件被系统清掉了，就要重新下，不能打开一个不存在的路径。
    std::fs::remove_file(world.downloaded(&file, "a.pdf")).unwrap();
    assert_eq!(
        actions_of(&world, &file).await.tap,
        FileCardAction::Download
    );
}

#[tokio::test]
async fn a_failed_download_can_be_tried_again() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");
    let info = info(&world, file.clone()).await;
    world.core.lock().files.clear();
    world
        .phone()
        .dispatch(Action::DownloadFile { id: file.clone() })
        .unwrap();
    world
        .wait_transfer(|t| matches!(t.state, TransferState::Failed { .. }))
        .await;

    let actions = world.phone().file_card_actions(info);
    assert_eq!(actions.tap, FileCardAction::Download);
    assert!(matches!(
        actions.download.unwrap().state,
        TransferState::Failed {
            reason: FileFailure::FileUnknown,
            ..
        }
    ));
}

#[tokio::test]
async fn each_transfer_says_which_buttons_it_has() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");
    let done = download(&world, &file).await;
    assert_eq!(
        done.actions,
        [
            TransferAction::Open,
            TransferAction::Share,
            TransferAction::DismissDownloaded
        ]
    );
    assert_eq!(world.phone().transfers().active, []);
    assert_eq!(world.phone().transfers().downloaded, [done]);

    let held = world
        .core
        .lock()
        .serve("b.pdf", &source_bytes(4 * MIB), "v1");
    world.core.lock().hold_download_after = Some(MIB);
    world
        .phone()
        .dispatch(Action::DownloadFile { id: held })
        .unwrap();
    let running = world.phone().transfers().active.pop().unwrap();
    assert_eq!(running.actions, [TransferAction::Cancel]);
}

fn room_upload(source: &Path) -> Action {
    Action::UploadFile {
        source: UploadSource::Path {
            path: source.to_str().unwrap().to_owned(),
        },
        name: "a.mp4".to_owned(),
        mime_type: None,
        room: Some("!bot:home".to_owned()),
    }
}

/// 这里的聊天没登录，发不出去。文件已经传完：不算失败，不重传，也不一直试。
#[tokio::test]
async fn a_link_that_was_not_sent_is_marked_and_only_the_link_is_resent() {
    let mut world = World::start().await;
    let bytes = source_bytes(MIB);
    let source = world.write("a.mp4", &bytes);
    world.phone().dispatch(room_upload(&source)).unwrap();
    let transfer = world
        .wait_transfer(|t| matches!(t.state, TransferState::LinkNotSent { .. }))
        .await;
    let file = transfer.file.clone().expect("字节传完了");
    assert_eq!(transfer.transferred_bytes, bytes.len() as u64);
    assert_eq!(
        transfer.actions,
        [TransferAction::ResendLink, TransferAction::StopSending]
    );
    assert_eq!(transfer.room.as_deref(), Some("!bot:home"));
    assert_eq!(
        world.core.lock().grants,
        [(file.to_string(), "!bot:home".to_owned())]
    );
    assert_eq!(world.phone().transfers().active.len(), 1, "等主人处理");

    // 不自动再试，App 重启也一样。
    world.restart_phone().await;
    let after_restart = world
        .wait_transfer(|t| matches!(t.state, TransferState::LinkNotSent { .. }))
        .await;
    assert_eq!(world.core.lock().grants.len(), 1);

    // 原文件删了、核心的 tus 也不认这个编号了都不要紧：字节已经传完。
    std::fs::remove_file(&source).unwrap();
    world.core.lock().uploads.clear();
    world
        .phone()
        .dispatch(Action::ResendFileLink {
            id: after_restart.id.clone(),
        })
        .unwrap();
    world.wait_core(|core| core.grants.len() == 2).await;
    world
        .wait_transfer(|t| {
            t.id == after_restart.id && matches!(t.state, TransferState::LinkNotSent { .. })
        })
        .await;
    assert_eq!(world.core.lock().patches.len(), 1, "字节不重传");
}

#[tokio::test]
async fn a_failed_grant_also_keeps_the_upload() {
    let world = World::start().await;
    world.core.lock().grant_fails_with = Some(FileFailure::ServiceUnavailable);
    let source = world.write("a.mp4", &source_bytes(1024));
    world.phone().dispatch(room_upload(&source)).unwrap();
    let transfer = world
        .wait_transfer(|t| matches!(t.state, TransferState::LinkNotSent { .. }))
        .await;
    assert!(matches!(
        transfer.state,
        TransferState::LinkNotSent {
            reason: FileFailure::ServiceUnavailable,
            ..
        }
    ));
    // 这样清掉不碰服务器上的文件，也不碰手机上的原文件。
    world
        .phone()
        .dispatch(Action::DismissTransfer {
            id: transfer.id,
            delete_local_file: false,
        })
        .unwrap();
    assert!(source.exists());
    let core = world.core.lock();
    assert!(core.terminated.is_empty() && core.deleted.is_empty());
}

/// 字节传完后的“不发了”：文件留在服务器上，tus 也不发 DELETE。
#[tokio::test]
async fn stopping_after_the_bytes_are_up_leaves_the_file_on_the_server() {
    let world = World::start().await;
    let source = world.write("a.mp4", &source_bytes(1024));
    world.phone().dispatch(room_upload(&source)).unwrap();
    let transfer = world
        .wait_transfer(|t| matches!(t.state, TransferState::LinkNotSent { .. }))
        .await;
    world
        .phone()
        .dispatch(Action::CancelTransfer {
            id: transfer.id.clone(),
        })
        .unwrap();
    assert!(world.phone().transfers().all().next().is_none());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let core = world.core.lock();
    assert!(core.terminated.is_empty() && core.deleted.is_empty());
    assert_eq!(core.uploaded_bytes(), 1024);
}

#[tokio::test]
async fn only_a_link_that_was_not_sent_can_be_resent() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");
    let done = download(&world, &file).await;
    assert!(matches!(
        world
            .phone()
            .dispatch(Action::ResendFileLink { id: done.id }),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[tokio::test]
async fn dismissing_only_the_record_keeps_the_file_openable() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");
    let done = download(&world, &file).await;
    world
        .phone()
        .dispatch(Action::DismissTransfer {
            id: done.id,
            delete_local_file: false,
        })
        .unwrap();
    assert_eq!(world.phone().transfers().downloaded, []);
    let path = world.downloaded(&file, "a.pdf");
    assert_eq!(std::fs::read(&path).unwrap(), b"abc");
    assert_eq!(actions_of(&world, &file).await.tap, open_at(&path));
}

#[tokio::test]
async fn dismissing_with_the_file_frees_the_phone() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");
    let done = download(&world, &file).await;
    world
        .phone()
        .dispatch(Action::DismissTransfer {
            id: done.id,
            delete_local_file: true,
        })
        .unwrap();
    assert_eq!(world.phone().transfers().downloaded, []);
    let path = world.downloaded(&file, "a.pdf");
    assert!(!path.parent().unwrap().exists());
    assert_eq!(
        actions_of(&world, &file).await.tap,
        FileCardAction::Download
    );
    assert!(world.core.lock().deleted.is_empty(), "服务器上的文件不动");
}

#[tokio::test]
async fn the_original_of_an_upload_is_not_ours_to_delete() {
    let world = World::start().await;
    let source = world.write("a.mp4", b"abc");
    world
        .phone()
        .dispatch(Action::UploadFile {
            source: UploadSource::Path {
                path: source.to_str().unwrap().to_owned(),
            },
            name: "a.mp4".to_owned(),
            mime_type: None,
            room: None,
        })
        .unwrap();
    let done = world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;
    assert_eq!(
        world.phone().transfers().uploaded,
        std::slice::from_ref(&done)
    );
    assert!(matches!(
        world.phone().dispatch(Action::DismissTransfer {
            id: done.id,
            delete_local_file: true,
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(source.exists());
}

/// 下载完的是主人手机上的文件，不因为下得多就自己从列表里消失：消失了就没有入口清理了。上传完的只留最近的。
#[tokio::test]
async fn finished_downloads_stay_until_the_owner_clears_them() {
    let world = World::start().await;
    let source = world.write("a.mp4", b"abc");
    for _ in 0..52 {
        world
            .phone()
            .dispatch(Action::UploadFile {
                source: UploadSource::Path {
                    path: source.to_str().unwrap().to_owned(),
                },
                name: "a.mp4".to_owned(),
                mime_type: None,
                room: None,
            })
            .unwrap();
        world
            .wait_until(|| world.phone().transfers().active.is_empty())
            .await;
    }
    assert!(world.phone().transfers().uploaded.len() < 52);

    for seed in 0..52 {
        let file = world
            .core
            .lock()
            .serve(&format!("{seed}.pdf"), b"abc", "v1");
        download(&world, &file).await;
    }
    assert_eq!(world.phone().transfers().downloaded.len(), 52);
}

/// iOS 上 App 更新后容器的路径会变，下载的文件要跟着找得到。
#[tokio::test]
async fn downloads_are_found_after_the_data_folder_moves() {
    let mut world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");
    download(&world, &file).await;

    let (phone, moved) = world.move_phone_data("phone-after-update");
    let view = phone.transfers();
    let destination = view.downloaded[0].destination.as_deref().unwrap();
    assert!(Path::new(destination).starts_with(&moved), "{destination}");
    assert_eq!(std::fs::read(destination).unwrap(), b"abc");
}

async fn playback_problem(world: &World, file: wonton_client::FileId) -> FileProblem {
    world.phone().file_playback_problem(file).await
}

#[tokio::test]
async fn a_player_failure_with_a_healthy_home_is_the_format() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.mp4", b"abc", "v1");
    let problem = playback_problem(&world, file).await;
    assert_eq!(problem.reason, FileFailure::UnsupportedFormat);
}

#[tokio::test]
async fn a_player_failure_with_a_core_error_says_so() {
    let world = World::start().await;
    let problem = playback_problem(&world, id(9)).await;
    assert_eq!(problem.reason, FileFailure::FileUnknown);
}

#[tokio::test]
async fn a_player_failure_without_a_home_is_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let phone = wonton_client::ClientCore::with_network(
        dir.path().to_str().unwrap(),
        wonton_client::Network::DirectOnly,
    )
    .unwrap();
    let problem = phone.file_playback_problem(id(1)).await;
    assert_eq!(problem.reason, FileFailure::TransportUnavailable);
}

/// 放的时候断了，等主人看到时隧道又好了：不能说成格式不支持。
#[tokio::test]
async fn a_connection_lost_while_playing_is_remembered() {
    let mut world = World::start().await;
    let file = world.core.lock().serve("a.mp4", b"abc", "v1");
    let url = world.phone().file_stream_url(file.clone()).await.unwrap();

    let port = world.stop_server();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let answer = client.get(&url).send().await.unwrap();
    assert!(!answer.status().is_success());
    world.start_server(port).await;

    let problem = playback_problem(&world, file).await;
    assert_eq!(problem.reason, FileFailure::TransportUnavailable);
}

#[tokio::test]
async fn players_asking_at_the_same_time_get_one_address() {
    let world = World::start().await;
    let (first, second) = world.core.lock().serve_two();
    let (one, other) = tokio::join!(
        world.phone().file_stream_url(first),
        world.phone().file_stream_url(second)
    );
    let port = |url: String| url::Url::parse(&url).unwrap().port();
    assert_eq!(port(one.unwrap()), port(other.unwrap()));
}

/// 发到别的房间的上传，进行中的只在目标房间列。
#[tokio::test]
async fn a_running_upload_is_listed_only_in_its_room() {
    let world = World::start().await;
    let source = world.write("a.mp4", &source_bytes(1024));
    world.core.lock().hold_patch_after = Some(512);
    world.phone().dispatch(room_upload(&source)).unwrap();
    world.wait_core(|core| core.uploaded_bytes() >= 512).await;
    let elsewhere = world.phone().room_transfers("!other:home".to_owned());
    assert!(elsewhere.here.is_empty() && elsewhere.elsewhere.is_empty());
    assert_eq!(
        world
            .phone()
            .room_transfers("!bot:home".to_owned())
            .here
            .len(),
        1
    );
}

/// 出了问题的每个房间都列，不然主人不知道 Bot 没收到。这里的聊天没登录，发不出去，也没有房间列表。
#[tokio::test]
async fn an_upload_whose_link_was_not_sent_is_listed_outside_its_room() {
    let world = World::start().await;
    let source = world.write("a.mp4", &source_bytes(1024));
    world.phone().dispatch(room_upload(&source)).unwrap();
    let transfer = world
        .wait_transfer(|t| matches!(t.state, TransferState::LinkNotSent { .. }))
        .await;
    let elsewhere = world.phone().room_transfers("!other:home".to_owned());
    assert!(elsewhere.here.is_empty());
    assert_eq!(elsewhere.elsewhere[0].transfer.id, transfer.id);
    assert_eq!(elsewhere.elsewhere[0].room_name, None);
    let home = world.phone().room_transfers("!bot:home".to_owned());
    assert_eq!(home.here[0].id, transfer.id);
    assert!(home.elsewhere.is_empty());
}
