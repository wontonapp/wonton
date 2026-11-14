//! 主人看到什么、能点什么（ADR-0020）：卡片上的按钮、链接没发出和重发、清理下载、播放失败的原因。只经由导出给原生壳的接口驱动。

use std::path::Path;
use std::time::Duration;

use crate::files_support::{Served, World, id, source_bytes};
use wonton_client::{
    Action, CoreError, FileCard, FileCardAction, FileCardActions, FileFailure, FileInfo,
    FileProblem, FileSource, TransferAction, TransferState, UploadSource,
};
use wonton_protocol::FileAction;

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
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile { id: file.clone() },
        })
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
    let mut world = World::start().await;
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
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile { id: file.clone() },
        })
        .unwrap();
    let during = actions_of(&world, &file).await;
    assert_eq!(during.tap, FileCardAction::Play, "下载的时候也能放");
    assert_eq!(during.open_elsewhere, Some(FileCardAction::Wait));
    assert!(during.download.is_some());

    // 下完了就用手上这份：不用走隧道，走中继也不用等。半截的接着下（假的核心停住不发了，重开手机走续传）。
    let part = world.downloaded(&file, "a.mp4.part");
    world
        .wait_until(|| part.metadata().is_ok_and(|meta| meta.len() == MIB as u64))
        .await;
    world.restart_phone().await;
    let done = world
        .wait_transfer(|t| t.file.as_ref() == Some(&file) && t.state == TransferState::Done)
        .await;
    assert_eq!(
        std::fs::read(world.downloaded(&file, "a.mp4")).unwrap(),
        source_bytes(4 * MIB),
        "接上的半截和先传的拼成完整的"
    );
    let after = actions_of(&world, &file).await;
    assert_eq!(after.tap, open_at(&world.downloaded(&file, "a.mp4")));
    assert_eq!(after.open_elsewhere, None, "点卡片就是打开，不用另给按钮");
    assert_eq!(after.download, Some(done));
}

/// 不是音视频的文件：下载后交给系统打开。
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
}

/// 文件被系统清掉了，就要重新下，不能打开一个不存在的路径。
#[tokio::test]
async fn a_downloaded_file_missing_from_disk_is_offered_again() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");
    download(&world, &file).await;

    std::fs::remove_file(world.downloaded(&file, "a.pdf")).unwrap();
    assert_eq!(
        actions_of(&world, &file).await.tap,
        FileCardAction::Download
    );
}

/// 没连上家里的服务器，手机上有这份文件就照常给卡片：打开、分享都用不上网。选过“只清记录”的也算。
#[tokio::test]
async fn a_downloaded_file_still_has_a_card_without_the_connection() {
    let mut world = World::start().await;
    let kept = world.core.lock().serve("kept.pdf", b"abcd", "v1");
    download(&world, &kept).await;
    let cleared = world.core.lock().serve("cleared.pdf", b"abc", "v1");
    let record = download(&world, &cleared).await;
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DismissTransfer {
                id: record.id,
                delete_local_file: false,
            },
        })
        .unwrap();

    // 家里的服务器没了，App 重开：连不上，手机上的文件还在。
    world.stop_phone();
    world.stop_server();
    world.start_phone_without_home().await;
    assert_eq!(world.phone().home().proxy, None);
    for (file, name, source) in [
        (&kept, "kept.pdf", FileSource::Device),
        // 记录清掉了：名字和大小从下载目录里的文件来，来源问不到了。
        (&cleared, "cleared.pdf", FileSource::Unknown),
    ] {
        let card = world.phone().file_card(file.clone()).await;
        let FileCard::Ready { info } = card else {
            panic!("连不上也该有卡片：{card:?}");
        };
        assert_eq!(info.name, name);
        assert_eq!(info.source, source);
        let actions = world.phone().file_card_actions(info);
        assert_eq!(actions.tap, open_at(&world.downloaded(file, name)));
        assert_eq!(actions.open_elsewhere, None);
    }
}

/// 服务器上的文件改过后，手机上这份就是旧的：不再直接打开，给重新下载；重下完打开的就是新的。
#[tokio::test]
async fn a_downloaded_file_that_changed_at_home_is_downloaded_again() {
    let mut world = World::start().await;
    let file = world.core.lock().serve("a.txt", b"old-content", "v1");
    download(&world, &file).await;
    world
        .core
        .lock()
        .files
        .insert(file.to_string(), Served::new("a.txt", b"new-content", "v2"));

    // 重启后卡片重新问一遍：没有缓存的旧答案。
    world.restart_phone().await;
    assert_eq!(
        actions_of(&world, &file).await.tap,
        FileCardAction::Download,
        "打开是旧内容，改给重新下载"
    );

    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile { id: file.clone() },
        })
        .unwrap();
    world
        .wait_transfer(|t| t.file.as_ref() == Some(&file) && t.state == TransferState::Done)
        .await;
    assert_eq!(
        actions_of(&world, &file).await.tap,
        open_at(&world.downloaded(&file, "a.txt"))
    );
    assert_eq!(
        std::fs::read(world.downloaded(&file, "a.txt")).unwrap(),
        b"new-content"
    );
}

/// 清理、取消、改名后重下，`downloads/<编号>/` 里不留没人认领的东西。
#[tokio::test]
async fn cleaning_up_a_download_leaves_no_folder_behind() {
    let world = World::start().await;
    let folder = |file: &wonton_client::FileId| {
        world
            .downloaded(file, "a.mov")
            .parent()
            .unwrap()
            .to_path_buf()
    };

    // 取消下载：半截的连目录一起删。
    let file = world
        .core
        .lock()
        .serve("a.mov", &source_bytes(4 * MIB), "v1");
    world.core.lock().hold_download_after = Some(MIB);
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile { id: file.clone() },
        })
        .unwrap();
    let part = world.downloaded(&file, "a.mov.part");
    world
        .wait_until(|| part.metadata().is_ok_and(|meta| meta.len() == MIB as u64))
        .await;
    let id = world
        .phone()
        .transfers()
        .active
        .iter()
        .find(|t| t.file.as_ref() == Some(&file))
        .unwrap()
        .id
        .clone();
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::CancelTransfer { id },
        })
        .unwrap();
    world.wait_until(|| !folder(&file).exists()).await;

    // 带删除的清理：整个目录一起删。
    world.core.lock().hold_download_after = None;
    let file = world.core.lock().serve("a.mov", &source_bytes(MIB), "v1");
    download(&world, &file).await;
    assert!(folder(&file).exists());
    let id = world.phone().transfers().downloaded[0].id.clone();
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DismissTransfer {
                id,
                delete_local_file: true,
            },
        })
        .unwrap();
    world.wait_until(|| !folder(&file).exists()).await;

    // 服务器上的文件改了名，重下一次：旧名字的文件没有入口，一起清掉。
    let file = world.core.lock().serve("old.mov", b"abc", "v1");
    download(&world, &file).await;
    world
        .core
        .lock()
        .files
        .insert(file.to_string(), Served::new("new.mov", b"abcdef", "v2"));
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile { id: file.clone() },
        })
        .unwrap();
    world
        .wait_transfer(|t| t.file.as_ref() == Some(&file) && t.state == TransferState::Done)
        .await;
    let left: Vec<_> = std::fs::read_dir(folder(&file))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["new.mov"], "旧名字的文件没有入口，不留着");
    assert_eq!(
        std::fs::read(world.downloaded(&file, "new.mov")).unwrap(),
        b"abcdef"
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
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile { id: file.clone() },
        })
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
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile { id: held },
        })
        .unwrap();
    let running = world.phone().transfers().active.pop().unwrap();
    assert_eq!(running.actions, [TransferAction::Cancel]);
}

fn room_upload(source: &Path) -> Action {
    Action::Files {
        action: wonton_protocol::FileAction::UploadFile {
            source: UploadSource::Path {
                path: source.to_str().unwrap().to_owned(),
            },
            name: "a.mp4".to_owned(),
            mime_type: None,
            room: Some("!bot:home".to_owned()),
        },
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
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::ResendFileLink {
                id: after_restart.id.clone(),
            },
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
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DismissTransfer {
                id: transfer.id,
                delete_local_file: false,
            },
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
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::CancelTransfer {
                id: transfer.id.clone(),
            },
        })
        .unwrap();
    assert!(world.phone().transfers().all().next().is_none());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let core = world.core.lock();
    assert!(core.terminated.is_empty() && core.deleted.is_empty());
    assert_eq!(core.uploaded_bytes(), 1024);
}

/// 不欠链接的传输上点重发：不算错（壳不弹窗），什么都不做；再跑一遍会重复副作用（带房间的上传多发一条链接）。
#[tokio::test]
async fn only_a_link_that_was_not_sent_can_be_resent() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");
    let done = download(&world, &file).await;
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::ResendFileLink {
                id: done.id.clone(),
            },
        })
        .unwrap();
    assert_eq!(world.phone().transfers().downloaded, [done], "什么都不做");
}

#[tokio::test]
async fn dismissing_only_the_record_keeps_the_file_openable() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.pdf", b"abc", "v1");
    let done = download(&world, &file).await;
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DismissTransfer {
                id: done.id,
                delete_local_file: false,
            },
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
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DismissTransfer {
                id: done.id,
                delete_local_file: true,
            },
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
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::UploadFile {
                source: UploadSource::Path {
                    path: source.to_str().unwrap().to_owned(),
                },
                name: "a.mp4".to_owned(),
                mime_type: None,
                room: None,
            },
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
        world.phone().dispatch(Action::Files {
            action: FileAction::DismissTransfer {
                id: done.id,
                delete_local_file: true,
            }
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
            .dispatch(Action::Files {
                action: wonton_protocol::FileAction::UploadFile {
                    source: UploadSource::Path {
                        path: source.to_str().unwrap().to_owned(),
                    },
                    name: "a.mp4".to_owned(),
                    mime_type: None,
                    room: None,
                },
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

/// 播放的这一趟核心报了错：壳问起来按当时说。等问的时候已经恢复了的话，再问一遍会把
/// 核心报过的错说成“放不了这个格式”。
#[tokio::test]
async fn a_playback_failure_names_what_the_core_said_at_the_time() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.mp4", b"abc", "v1");
    let url = world.phone().file_stream_url(file.clone()).await.unwrap();

    world.core.lock().services_down = true;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let answer = client.get(&url).send().await.unwrap();
    assert_eq!(answer.status(), 503);
    // 等壳问原因的时候家里已经恢复了。
    world.core.lock().services_down = false;

    let problem = playback_problem(&world, file).await;
    assert_eq!(problem.reason, FileFailure::ServiceUnavailable);
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
