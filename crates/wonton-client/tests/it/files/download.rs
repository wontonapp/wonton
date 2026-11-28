//! 下载：文件名清洗后落在 App 目录里，传完才出现，重下替换，失败说清哪边。

use super::*;

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
                action: wonton_protocol::FileAction::DownloadFile {
                    id: file.clone(),
                    room: "!bot:home".into(),
                },
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
async fn a_download_appears_in_the_app_folder_only_when_complete() {
    let world = World::start().await;
    let bytes = source_bytes(3 * MIB);
    let file = world.core.lock().serve("a.mov", &bytes, "v1");
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile {
                id: file.clone(),
                room: "!bot:home".into(),
            },
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
        action: wonton_protocol::FileAction::DownloadFile {
            id: file,
            room: "!bot:home".into(),
        },
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

#[tokio::test]
async fn download_failures_say_which_side_failed() {
    let world = World::start().await;
    let unknown = id(9);
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DownloadFile {
                id: unknown.clone(),
                room: "!bot:home".into(),
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
            action: wonton_protocol::FileAction::DownloadFile {
                id: file,
                room: "!bot:home".into(),
            },
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
        action: wonton_protocol::FileAction::DownloadFile {
            id: file,
            room: "!bot:home".into(),
        },
    };
    world.phone().dispatch(download.clone()).unwrap();
    assert!(matches!(
        world.phone().dispatch(download),
        Err(CoreError::InvalidInput { .. })
    ));
}
