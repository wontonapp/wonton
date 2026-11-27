//! 传输列表：取消、忽略、未知编号，列表读坏和没有家里时如实报不可用。

use super::*;

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
