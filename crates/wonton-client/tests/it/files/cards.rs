//! 文件卡片和缩略图：卡片缓存到文件删除，失败的下次重问，缩略图回字节或原因。

use super::*;

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
