//! 原生相册页（ADR-0113 第 9 节）的行为测试：只经导出给原生壳的接口驱动，隧道对面的假核心按提供方的形状回
//! `/_wonton/api/media/library/…`。断言看 ViewModel，也看假核心收到的请求和本机回环给出的字节。

use std::time::Duration;

use axum::http::Method;
use wonton_client::{Action, CoreError};
use wonton_protocol::media::MediaFailureCode;
use wonton_protocol::media::library::AlbumOwner;
use wonton_protocol::media::library_view::{
    PhotoEdit, PhotoEditKind, PhotoRendition, PhotoSearch, PhotosAction, PhotosViewModel,
};

use crate::photos_support::{album, asset, envelope};
use crate::plugins::rig::{Canned, FakeHome, Rig, WAIT};

const TIMELINE: &str = "/_wonton/api/media/library/timeline";
pub(crate) const ALBUMS: &str = "/_wonton/api/media/library/albums";

/// 东八区 2026-11-27 00:30、2026-11-26 21:33、2026-11-26 18:46。
const LATE_27: u64 = 1_795_710_600_000;
const NIGHT_26: u64 = 1_795_700_000_000;
const EVENING_26: u64 = 1_795_690_000_000;

pub(crate) async fn photos(
    rig: &Rig,
    what: &str,
    mut check: impl FnMut(&PhotosViewModel) -> bool,
) -> PhotosViewModel {
    let subscription = rig.app.subscribe_photos();
    let waiting = async {
        while let Some(view) = subscription.next().await {
            if check(&view) {
                return view;
            }
        }
        panic!("订阅提前结束");
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("{what}：超时，当前：{:#?}", rig.app.photos()))
}

pub(crate) fn send(rig: &Rig, action: PhotosAction) -> Result<(), CoreError> {
    rig.app.dispatch(Action::Photos { action })
}

pub(crate) fn open(rig: &Rig) {
    open_with(rig, None);
}

fn open_with(rig: &Rig, source: Option<&str>) {
    send(
        rig,
        PhotosAction::Open {
            utc_offset_minutes: 480,
            source: source.map(str::to_owned),
        },
    )
    .unwrap();
}

pub(crate) fn november(rig: &Rig) {
    rig.core.photos.month(
        "2026-11",
        vec![
            asset("a", LATE_27),
            asset("b", NIGHT_26),
            asset("c", EVENING_26),
        ],
    );
}

fn ids(view: &PhotosViewModel) -> Vec<String> {
    view.timeline.months[0]
        .days
        .iter()
        .flat_map(|day| day.tiles.iter().map(|tile| tile.id.clone()))
        .collect()
}

/// 打开相册页、打开十一月，等第一页到。
pub(crate) async fn november_open(rig: &Rig) -> PhotosViewModel {
    rig.connected().await;
    open(rig);
    november_month(rig).await
}

/// 相册页已打开：等时间线到，打开十一月，等第一页到。
async fn november_month(rig: &Rig) -> PhotosViewModel {
    photos(rig, "时间线", |view| !view.timeline.months.is_empty()).await;
    send(
        rig,
        PhotosAction::OpenMonth {
            month: "2026-11".into(),
        },
    )
    .unwrap();
    photos(rig, "十一月第一页", |view| {
        let month = &view.timeline.months[0];
        month.opened && !month.loading && !month.days.is_empty()
    })
    .await
}

#[tokio::test]
async fn a_month_opens_page_by_page_grouped_by_local_day() {
    let rig = Rig::new().await;
    november(&rig);
    let view = november_open(&rig).await;
    let month = &view.timeline.months[0];
    assert_eq!(month.count, 3);
    assert!(month.has_more);
    let dates: Vec<_> = month.days.iter().map(|day| day.date.as_str()).collect();
    assert_eq!(dates, ["2026-11-27", "2026-11-26"]);

    send(
        &rig,
        PhotosAction::LoadMoreMonth {
            month: "2026-11".into(),
        },
    )
    .unwrap();
    let view = photos(&rig, "第二页", |view| !view.timeline.months[0].has_more).await;
    assert_eq!(ids(&view), ["a", "b", "c"]);
    assert_eq!(
        view.timeline.months[0].days.len(),
        2,
        "同一天接在那一天后面"
    );
    assert_eq!(
        rig.core
            .seen(&Method::GET, &format!("{TIMELINE}/2026-11?cursor=2"))
            .len(),
        1,
        "第二页带上第一页回的游标"
    );
    // 最后一页之后再翻不发请求。
    send(
        &rig,
        PhotosAction::LoadMoreMonth {
            month: "2026-11".into(),
        },
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let pages = rig
        .core
        .all_seen()
        .into_iter()
        .filter(|seen| seen.path.starts_with(&format!("{TIMELINE}/2026-11")))
        .count();
    assert_eq!(pages, 2);
}

#[tokio::test]
async fn albums_are_split_by_owner_and_open_page_by_page() {
    let rig = Rig::new().await;
    november(&rig);
    rig.core
        .photos
        .album(album("al1", "海边", AlbumOwner::Me), &["a", "b", "c"]);
    rig.core
        .photos
        .album(album("al2", "家", AlbumOwner::Household), &["a"]);
    rig.core.photos.album(
        album("al3", "院子", AlbumOwner::Person("mom".into())),
        &["b"],
    );
    rig.core
        .photos
        .album(album("al4", "?", AlbumOwner::Other("guest".into())), &[]);
    rig.connected().await;
    open(&rig);
    let view = photos(&rig, "相册列表", |view| !view.albums.mine.is_empty()).await;
    let titles = |rows: &[wonton_protocol::media::library_view::PhotoAlbumRow]| {
        rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>()
    };
    assert_eq!(titles(&view.albums.mine), ["al1"]);
    assert_eq!(titles(&view.albums.household), ["al2"]);
    assert_eq!(titles(&view.albums.shared_with_me), ["al3", "al4"]);
    assert_eq!(view.albums.mine[0].count, 3);

    send(&rig, PhotosAction::OpenAlbum { id: "al1".into() }).unwrap();
    let view = photos(&rig, "相册第一页", |view| {
        view.album.as_ref().is_some_and(|album| !album.page.loading)
    })
    .await;
    let open = view.album.unwrap();
    assert_eq!(open.album.unwrap().title, "海边");
    assert_eq!(open.page.tiles.len(), 2);
    assert!(open.page.has_more);
    send(&rig, PhotosAction::LoadMoreAlbum).unwrap();
    let view = photos(&rig, "相册第二页", |view| {
        view.album
            .as_ref()
            .is_some_and(|album| !album.page.has_more)
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
    assert_eq!(tiles, ["a", "b", "c"]);
}

#[tokio::test]
async fn search_sends_the_query_and_detail_fills_in() {
    let rig = Rig::new().await;
    november(&rig);
    rig.core.photos.person("p1", "奶奶");
    rig.core.photos.found(vec![
        asset("c", EVENING_26),
        asset("a", LATE_27),
        asset("b", NIGHT_26),
    ]);
    rig.connected().await;
    open(&rig);
    let view = photos(&rig, "人物", |view| !view.people.people.is_empty()).await;
    assert_eq!(view.people.people[0].name, "奶奶");

    let empty = send(
        &rig,
        PhotosAction::Search {
            query: PhotoSearch {
                text: Some("  ".into()),
                ..PhotoSearch::default()
            },
        },
    );
    assert!(
        matches!(empty, Err(CoreError::InvalidInput { .. })),
        "{empty:?}"
    );

    let query = PhotoSearch {
        text: Some("海边 日落".into()),
        people: vec!["p1".into()],
        ..PhotoSearch::default()
    };
    send(
        &rig,
        PhotosAction::Search {
            query: query.clone(),
        },
    )
    .unwrap();
    let view = photos(&rig, "搜索结果", |view| {
        view.search
            .as_ref()
            .is_some_and(|search| !search.page.loading)
    })
    .await;
    let search = view.search.unwrap();
    assert_eq!(search.query, query);
    // 按相关度排，不按时间重排。
    let order: Vec<_> = search
        .page
        .tiles
        .iter()
        .map(|tile| tile.id.as_str())
        .collect();
    assert_eq!(order, ["c", "a"]);
    let sent = rig
        .core
        .all_seen()
        .into_iter()
        .find(|seen| seen.path.starts_with("/_wonton/api/media/library/search?"))
        .expect("搜索发给核心");
    assert!(sent.path.contains("people=p1"), "{}", sent.path);
    assert!(
        sent.path
            .contains("q=%E6%B5%B7%E8%BE%B9%20%E6%97%A5%E8%90%BD"),
        "{}",
        sent.path
    );

    send(&rig, PhotosAction::LoadMoreSearch).unwrap();
    photos(&rig, "搜索第二页", |view| {
        view.search
            .as_ref()
            .is_some_and(|search| search.page.tiles.len() == 3)
    })
    .await;

    send(&rig, PhotosAction::OpenDetail { id: "a".into() }).unwrap();
    let view = photos(&rig, "详情", |view| {
        view.detail
            .as_ref()
            .is_some_and(|detail| detail.info.is_some())
    })
    .await;
    let detail = view.detail.unwrap();
    assert_eq!(detail.tile.unwrap().taken_at, LATE_27);
    let info = detail.info.unwrap();
    assert_eq!(info.width, Some(4032));
    assert_eq!(info.place, ["青岛"]);
}

#[tokio::test]
async fn edits_change_only_what_succeeded_and_list_what_did_not() {
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
            edit: PhotoEdit::Favorite {
                ids: vec!["a".into(), "b".into()],
                favorite: true,
            },
        },
    )
    .unwrap();
    let view = photos(&rig, "收藏结果", |view| {
        view.last_edit.is_some() && view.editing == 0
    })
    .await;
    let outcome = view.last_edit.clone().unwrap();
    assert_eq!(outcome.kind, PhotoEditKind::Favorite);
    assert_eq!(outcome.done, ["a"]);
    assert_eq!(outcome.failed.len(), 1);
    assert_eq!(outcome.failed[0].id, "b");
    assert_eq!(
        outcome.failed[0].failure.code,
        MediaFailureCode::PermissionDenied
    );
    let tiles: Vec<_> = view.timeline.months[0]
        .days
        .iter()
        .flat_map(|day| day.tiles.iter())
        .map(|tile| (tile.id.as_str(), tile.favorite))
        .collect();
    assert_eq!(tiles, [("a", true), ("b", false)]);
    assert_eq!(view.selected, ["b"], "没成的还选着，好再试");

    send(&rig, PhotosAction::DismissEditOutcome).unwrap();
    send(
        &rig,
        PhotosAction::Edit {
            edit: PhotoEdit::Trash {
                ids: vec!["a".into()],
            },
        },
    )
    .unwrap();
    let view = photos(&rig, "进回收站", |view| {
        view.last_edit
            .as_ref()
            .is_some_and(|edit| edit.kind == PhotoEditKind::Trash)
    })
    .await;
    assert_eq!(ids(&view), ["b"]);
    assert_eq!(view.timeline.months[0].count, 2);

    // 撤销：恢复后时间线重取，那一张回来。
    send(
        &rig,
        PhotosAction::Edit {
            edit: PhotoEdit::Restore {
                ids: vec!["a".into()],
            },
        },
    )
    .unwrap();
    let view = photos(&rig, "恢复", |view| {
        view.timeline.months.first().is_some_and(|month| {
            month.count == 3
                && !month.loading
                && month
                    .days
                    .iter()
                    .any(|day| day.tiles.iter().any(|tile| tile.id == "a"))
        })
    })
    .await;
    assert_eq!(view.last_edit.unwrap().done, ["a"]);
}

#[tokio::test]
async fn failures_keep_the_category_the_core_gave() {
    let rig = Rig::new().await;
    november(&rig);
    // 没连上：问不了用哪家，不发请求，按传输不可用报。
    open(&rig);
    let view = photos(&rig, "没连上", |view| view.sources.problem.is_some()).await;
    assert_eq!(
        view.sources.problem.unwrap().code,
        MediaFailureCode::TransportUnavailable
    );
    // 连上后自己补取。
    rig.connected().await;
    photos(&rig, "连上补取", |view| {
        view.timeline.problem.is_none() && !view.timeline.months.is_empty()
    })
    .await;

    let cases = [
        (
            403,
            envelope(MediaFailureCode::PermissionDenied, "not opened"),
            MediaFailureCode::PermissionDenied,
        ),
        (
            503,
            envelope(MediaFailureCode::PluginUnavailable, "down"),
            MediaFailureCode::PluginUnavailable,
        ),
        (
            502,
            envelope(MediaFailureCode::UpstreamError, "upstream 500"),
            MediaFailureCode::UpstreamError,
        ),
        (
            409,
            envelope(MediaFailureCode::SourceRequired, "two photo plugins"),
            MediaFailureCode::SourceRequired,
        ),
        (
            400,
            envelope(MediaFailureCode::Protocol, "bad month"),
            MediaFailureCode::Protocol,
        ),
        // 不是信封（核心或网关自己回的）：按状态码归类。
        (500, "boom".to_owned(), MediaFailureCode::UpstreamError),
        (403, "{}".to_owned(), MediaFailureCode::PermissionDenied),
        (404, String::new(), MediaFailureCode::MediaGone),
        (418, "teapot".to_owned(), MediaFailureCode::Protocol),
    ];
    for (status, body, expected) in cases {
        rig.core.answer(Method::GET, TIMELINE, Canned(status, body));
        send(&rig, PhotosAction::Refresh).unwrap();
        let view = photos(&rig, "失败类别", |view| {
            !view.timeline.loading && view.timeline.problem.is_some()
        })
        .await;
        assert_eq!(view.timeline.problem.unwrap().code, expected, "{status}");
        // 失败不清掉已有的分桶。
        assert_eq!(view.timeline.months.len(), 1);
        send(&rig, PhotosAction::Refresh).unwrap();
        photos(&rig, "恢复", |view| {
            !view.timeline.loading && view.timeline.problem.is_none()
        })
        .await;
    }
}

#[tokio::test]
async fn a_late_answer_from_the_previous_server_does_not_land() {
    let rig = Rig::new().await;
    rig.core
        .photos
        .album(album("old", "旧服务器的", AlbumOwner::Me), &[]);
    rig.connected().await;
    let held = rig.core.hold(Method::GET, ALBUMS);
    open(&rig);
    let deadline = tokio::time::Instant::now() + WAIT;
    while rig.core.seen(&Method::GET, ALBUMS).is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "相册请求没有出门");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let next = FakeHome::start(vec![rig.device()]).await;
    next.core
        .photos
        .album(album("new", "新服务器的", AlbumOwner::Me), &[]);
    rig.connect_to(next.ticket());
    let view = photos(&rig, "新服务器的相册", |view| {
        view.albums.mine.iter().any(|row| row.id == "new")
    })
    .await;
    assert!(view.albums.problem.is_none());
    held.send_replace(false);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let view = rig.app.photos();
    let mine: Vec<_> = view.albums.mine.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(mine, ["new"]);
    assert!(view.albums.problem.is_none(), "{:?}", view.albums.problem);
}

async fn get(url: &str, range: Option<&str>) -> reqwest::Response {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut request = client.get(url);
    if let Some(range) = range {
        request = request.header("range", range);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn bytes_come_through_the_loopback_with_range() {
    let rig = Rig::new().await;
    november(&rig);
    let original: Vec<u8> = (0..200u8).collect();
    rig.core.photos.bytes("a", "original", original.clone());
    rig.core.photos.bytes("p1", "person", vec![3; 16]);
    rig.connected().await;

    let url = rig
        .app
        .photo_url("a".into(), PhotoRendition::Original)
        .await
        .unwrap();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    let response = get(&url, Some("bytes=10-19")).await;
    assert_eq!(response.status(), 206);
    assert_eq!(
        response.headers()["content-range"].to_str().unwrap(),
        "bytes 10-19/200"
    );
    assert_eq!(response.bytes().await.unwrap().to_vec(), original[10..20]);
    let whole = get(&url, None).await;
    assert_eq!(whole.status(), 200);
    assert_eq!(whole.bytes().await.unwrap().to_vec(), original);

    // 令牌不对的拿不到。
    let token = url.split('/').nth(3).unwrap();
    assert_eq!(
        get(&url.replacen(token, "wrong", 1), None).await.status(),
        401
    );

    let face = rig.app.photo_person_url("p1".into()).await.unwrap();
    assert_eq!(
        get(&face, None).await.bytes().await.unwrap().to_vec(),
        vec![3; 16]
    );
    let missing = rig
        .app
        .photo_url("nope".into(), PhotoRendition::Preview)
        .await
        .unwrap();
    assert_eq!(get(&missing, None).await.status(), 404);
}

#[tokio::test]
async fn the_chosen_plugin_goes_with_every_request_and_switching_starts_over() {
    let rig = Rig::new().await;
    november(&rig);
    rig.core.photos.bytes("a", "thumbnail", vec![9; 8]);
    rig.connected().await;
    open_with(&rig, Some("app.example.photos"));
    november_month(&rig).await;
    let sent: Vec<String> = rig
        .core
        .all_seen()
        .into_iter()
        .map(|seen| seen.path)
        .filter(|path| path.starts_with("/_wonton/api/media/library"))
        .collect();
    assert!(!sent.is_empty());
    for path in &sent {
        assert!(path.contains("source=app.example.photos"), "{path}");
    }
    let thumbnail = rig
        .app
        .photo_url("a".into(), PhotoRendition::Thumbnail)
        .await
        .unwrap();
    assert!(
        thumbnail.contains("source=app.example.photos"),
        "{thumbnail}"
    );
    assert_eq!(
        get(&thumbnail, None).await.bytes().await.unwrap().to_vec(),
        vec![9; 8]
    );

    // 换一个插件：打开过的月不留，从头取。
    let held = rig.core.hold(Method::GET, TIMELINE);
    open_with(&rig, Some("app.example.other"));
    let view = rig.app.photos();
    assert!(view.timeline.months.is_empty(), "{view:#?}");
    held.send_replace(false);
    let view = photos(&rig, "另一个插件的时间线", |view| {
        !view.timeline.loading && !view.timeline.months.is_empty()
    })
    .await;
    assert!(!view.timeline.months[0].opened);
    assert!(
        rig.core
            .all_seen()
            .iter()
            .any(|seen| seen.path == format!("{TIMELINE}?source=app.example.other"))
    );
}

#[tokio::test]
async fn bytes_before_connecting_say_the_home_is_unreachable_in_the_loader() {
    let rig = Rig::new().await;
    // 回环地址本机就能给；连不上家里时，加载器取的时候拿到 503。
    let url = rig
        .app
        .photo_url("a".into(), PhotoRendition::Thumbnail)
        .await
        .unwrap();
    assert_eq!(get(&url, None).await.status(), 503);
}
