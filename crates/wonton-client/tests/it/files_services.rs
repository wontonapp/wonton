//! 家里的系统服务不可用（ADR-0020）：传输暂停而不是失败，说得清是服务的问题，服务恢复后接着传。整套服务重启时隧道通了而 tusd 还没起来，核心回
//! `service_unavailable`；tusd 真坏了时一直如此。

use crate::files_support::{World, source_bytes};
use wonton_client::{Action, Transfer, TransferAction, TransferState, UploadSource};

const MIB: usize = 1024 * 1024;

fn upload(source: &std::path::Path, room: Option<&str>) -> Action {
    Action::UploadFile {
        source: UploadSource::Path {
            path: source.to_str().unwrap().to_owned(),
        },
        name: "a.mp4".to_owned(),
        mime_type: None,
        room: room.map(str::to_owned),
    }
}

/// 核心真回过 503 之后的暂停：等 `Paused` 不够，`Store::open` 一启动就把进行中的改成暂停。
async fn paused_by_the_services(world: &World) -> Transfer {
    world.wait_core(|core| core.refused > 0).await;
    world
        .wait_transfer(|t| t.state == TransferState::Paused && t.paused_because.is_some())
        .await
}

async fn services_come_back(world: &World) -> Transfer {
    {
        let mut core = world.core.lock();
        core.services_down = false;
        core.patches_down = false;
    }
    world.phone().dispatch(Action::AppForegrounded).unwrap();
    let done = world
        .wait_transfer(|t| t.state == TransferState::Done)
        .await;
    assert_eq!(done.paused_because, None);
    done
}

#[tokio::test]
async fn an_upload_waits_out_the_services_when_asking_where_to_resume() {
    let mut world = World::start().await;
    let bytes = source_bytes(4 * MIB);
    let source = world.write("a.mp4", &bytes);
    world.core.lock().hold_patch_after = Some(MIB);
    world.phone().dispatch(upload(&source, None)).unwrap();
    world.wait_core(|core| core.uploaded_bytes() >= MIB).await;

    world.stop_phone();
    world.core.lock().services_down = true;
    world.start_phone().await;
    paused_by_the_services(&world).await;

    let done = services_come_back(&world).await;
    assert_eq!(
        world.core.lock().uploads[done.file.unwrap().as_str()].bytes,
        bytes
    );
}

#[tokio::test]
async fn an_upload_waits_out_the_services_while_sending_bytes() {
    let world = World::start().await;
    let bytes = source_bytes(MIB);
    let source = world.write("a.mp4", &bytes);
    world.core.lock().patches_down = true;
    world.phone().dispatch(upload(&source, None)).unwrap();
    paused_by_the_services(&world).await;

    let done = services_come_back(&world).await;
    assert_eq!(
        world.core.lock().uploads[done.file.unwrap().as_str()].bytes,
        bytes
    );
}

#[tokio::test]
async fn a_download_waits_out_the_services_when_asking_the_name() {
    let world = World::start().await;
    let bytes = source_bytes(1024);
    let file = world.core.lock().serve("a.mov", &bytes, "v1");
    world.core.lock().services_down = true;
    world
        .phone()
        .dispatch(Action::DownloadFile { id: file.clone() })
        .unwrap();
    paused_by_the_services(&world).await;

    services_come_back(&world).await;
    assert_eq!(
        std::fs::read(world.downloaded(&file, "a.mov")).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn a_download_waits_out_the_services_when_fetching_the_content() {
    let mut world = World::start().await;
    let bytes = source_bytes(2 * MIB);
    let file = world.core.lock().serve("a.mov", &bytes, "v1");
    world.core.lock().hold_download_after = Some(MIB);
    let part = world.downloaded(&file, "a.mov.part");
    world
        .phone()
        .dispatch(Action::DownloadFile { id: file.clone() })
        .unwrap();
    world
        .wait_until(|| part.metadata().is_ok_and(|meta| meta.len() == MIB as u64))
        .await;

    world.stop_phone();
    world.core.lock().services_down = true;
    let asked_names = world.core.lock().info_requests;
    world.start_phone().await;
    paused_by_the_services(&world).await;
    assert_eq!(
        world.core.lock().info_requests,
        asked_names,
        "文件名问过了，这次的 503 来自取内容"
    );

    services_come_back(&world).await;
    assert_eq!(
        std::fs::read(world.downloaded(&file, "a.mov")).unwrap(),
        bytes
    );
}

/// 断网的暂停不带说明：壳这时写“等连接恢复”。
#[tokio::test]
async fn a_pause_for_the_connection_does_not_blame_the_services() {
    let mut world = World::start().await;
    let source = world.write("a.mp4", &source_bytes(4 * MIB));
    world.core.lock().hold_patch_after = Some(MIB);
    world.phone().dispatch(upload(&source, None)).unwrap();
    world.wait_core(|core| core.uploaded_bytes() >= MIB).await;

    world.stop_server();
    let paused = world
        .wait_transfer(|t| t.state == TransferState::Paused)
        .await;
    assert_eq!(paused.paused_because, None);
}

/// 字节传完、停在发链接这一步：按钮是“不发了”，文件留在服务器上。
#[tokio::test]
async fn stopping_while_the_link_is_on_its_way_leaves_the_file_on_the_server() {
    let world = World::start().await;
    let source = world.write("a.mp4", &source_bytes(1024));
    world.core.lock().hold_grant = true;
    world
        .phone()
        .dispatch(upload(&source, Some("!bot:home")))
        .unwrap();
    let transfer = world
        .wait_transfer(|t| t.actions == [TransferAction::StopSending])
        .await;
    world
        .phone()
        .dispatch(Action::CancelTransfer { id: transfer.id })
        .unwrap();
    assert!(world.phone().transfers().all().next().is_none());
    let core = world.core.lock();
    assert!(core.terminated.is_empty() && core.deleted.is_empty());
    assert_eq!(core.uploaded_bytes(), 1024);
}
