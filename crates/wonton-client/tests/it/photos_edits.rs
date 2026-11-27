//! 相册页的写操作表和相册上的写操作（建、移出、共享）：只经导出给原生壳的接口驱动，断言看 ViewModel 和假核心
//! 收到的请求。

use std::time::Duration;

use axum::http::Method;
use wonton_protocol::media::MediaFailureCode;
use wonton_protocol::media::choices::SHARE_TARGETS_PATH;
use wonton_protocol::media::library::AlbumOwner;
use wonton_protocol::media::library_view::{PhotoEdit, PhotoEditKind, PhotosAction};

use crate::photos::{ALBUMS, november, november_open, open, photos, send};
use crate::photos_support::{album, envelope};
use crate::plugins::rig::{Canned, Rig, WAIT};

const LIBRARY: &str = "/_wonton/api/media/library";

/// 等假核心收到这个请求的次数超过 `before`。
async fn until_seen(rig: &Rig, path: &str, before: usize) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while rig.core.seen(&Method::GET, path).len() <= before {
        assert!(tokio::time::Instant::now() < deadline, "{path} 没有重取");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn edit_buttons_follow_what_the_plugin_accepts_and_say_why_when_unknown() {
    let rig = Rig::new().await;
    rig.connected().await;
    open(&rig);
    let view = photos(&rig, "写操作表", |view| view.actions.is_some()).await;
    // 假插件接全部七种，外加一个这一版不认识的：认识的都在（收藏拆成两个），不认识的不列。
    assert_eq!(view.actions.unwrap().len(), 8);
    assert_eq!(view.actions_problem, None);

    // 取不到就不确定：不出写操作的按钮，原因单独带出，时间线照常。
    rig.core.answer(
        Method::GET,
        LIBRARY,
        Canned(503, envelope(MediaFailureCode::PluginUnavailable, "down")),
    );
    send(&rig, PhotosAction::Refresh).unwrap();
    let view = photos(&rig, "写操作表没取成", |view| {
        view.actions.is_none() && view.actions_problem.is_some() && !view.timeline.loading
    })
    .await;
    assert_eq!(
        view.actions_problem.unwrap().code,
        MediaFailureCode::PluginUnavailable
    );
    assert_eq!(view.timeline.problem, None);

    // 重试取成：原因清掉，接着重取能共享给的家人（等它回来，后面好数请求）。
    let before = rig.core.seen(&Method::GET, SHARE_TARGETS_PATH).len();
    send(&rig, PhotosAction::Refresh).unwrap();
    photos(&rig, "重试取成", |view| {
        view.actions.is_some() && view.actions_problem.is_none()
    })
    .await;
    until_seen(&rig, SHARE_TARGETS_PATH, before).await;
    photos(&rig, "能共享给的家人", |view| {
        !view.share_targets.loading && !view.share_targets.people.is_empty()
    })
    .await;

    // 只读的源：空表，不是“不确定”；没有建相册和共享，家人名单也不留、不再去读。
    let asked = rig.core.seen(&Method::GET, SHARE_TARGETS_PATH).len();
    rig.core.answer(
        Method::GET,
        LIBRARY,
        Canned(200, r#"{"actions":[]}"#.to_owned()),
    );
    send(&rig, PhotosAction::Refresh).unwrap();
    let view = photos(&rig, "只读", |view| view.actions == Some(Vec::new())).await;
    assert!(view.share_targets.people.is_empty());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(rig.core.seen(&Method::GET, SHARE_TARGETS_PATH).len(), asked);
}

#[tokio::test]
async fn a_new_album_from_a_mixed_selection_lists_what_failed() {
    let rig = Rig::new().await;
    november(&rig);
    rig.core
        .photos
        .refuse("b", MediaFailureCode::PermissionDenied);
    november_open(&rig).await;
    send(
        &rig,
        PhotosAction::Select {
            ids: vec!["a".into(), "b".into()],
            selected: true,
        },
    )
    .unwrap();
    send(
        &rig,
        PhotosAction::Edit {
            edit: PhotoEdit::CreateAlbum {
                title: "海边".into(),
                ids: vec!["a".into(), "b".into()],
                share_with: vec!["mom".into()],
            },
        },
    )
    .unwrap();
    let view = photos(&rig, "建相册", |view| {
        view.last_edit.is_some() && view.editing == 0
    })
    .await;
    let outcome = view.last_edit.clone().unwrap();
    assert_eq!(outcome.kind, PhotoEditKind::CreateAlbum);
    assert_eq!(outcome.done, ["a"]);
    let failed: Vec<_> = outcome.failed.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(failed, ["b"]);
    assert_eq!(view.selected, ["b"], "没成的还选着，好再试");
    // 新相册进“我的”，建的时候就共享给了选的人（假核心照收到的请求记）。
    let created = outcome.album.expect("建成的相册");
    let row = view
        .albums
        .mine
        .iter()
        .find(|row| row.id == created.id)
        .expect("新相册在我的里");
    assert_eq!(row.title, "海边");
    assert_eq!(row.count, 1);
    assert!(row.shared);
    assert_eq!(row.members, ["mom"]);
}

#[tokio::test]
async fn sharing_or_removing_from_the_open_album_refetches_it() {
    let rig = Rig::new().await;
    november(&rig);
    rig.core
        .photos
        .album(album("al1", "海边", AlbumOwner::Me), &["a", "b"]);
    rig.connected().await;
    open(&rig);
    photos(&rig, "相册列表", |view| !view.albums.mine.is_empty()).await;
    send(&rig, PhotosAction::OpenAlbum { id: "al1".into() }).unwrap();
    photos(&rig, "打开相册", |view| {
        view.album
            .as_ref()
            .is_some_and(|album| !album.page.loading && album.page.tiles.len() == 2)
    })
    .await;

    let one = format!("{ALBUMS}/al1");
    let (list, page) = (
        rig.core.seen(&Method::GET, ALBUMS).len(),
        rig.core.seen(&Method::GET, &one).len(),
    );
    send(
        &rig,
        PhotosAction::Edit {
            edit: PhotoEdit::ShareAlbum {
                album: "al1".into(),
                people: vec!["mom".into()],
            },
        },
    )
    .unwrap();
    let view = photos(&rig, "共享", |view| {
        view.last_edit
            .as_ref()
            .is_some_and(|edit| edit.kind == PhotoEditKind::ShareAlbum)
    })
    .await;
    let row = view.album.unwrap().album.unwrap();
    assert!(row.shared);
    assert_eq!(row.members, ["mom"]);
    until_seen(&rig, ALBUMS, list).await;
    until_seen(&rig, &one, page).await;

    send(
        &rig,
        PhotosAction::Edit {
            edit: PhotoEdit::RemoveFromAlbum {
                album: "al1".into(),
                ids: vec!["a".into()],
            },
        },
    )
    .unwrap();
    let view = photos(&rig, "移出相册", |view| {
        view.albums.mine.first().is_some_and(|row| row.count == 1)
    })
    .await;
    let tiles: Vec<_> = view
        .album
        .unwrap()
        .page
        .tiles
        .into_iter()
        .map(|tile| tile.id)
        .collect();
    assert_eq!(tiles, ["b"]);
}
