//! 曲库浏览的行为：对着本机一台假的 OpenSubsonic 曲库开页、翻页、搜索、加星，断言壳拿到的视图。
//! 假曲库照 Navidrome 0.64.2 实测的行为：代理头里的人没账号时回 40，打开一次 `/app/` 后才有。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use url::Url;
use wonton_protocol::music::{
    AlbumSort, MUSIC_PAGE_SIZE, MusicAction, MusicContent, MusicFailure, MusicPage,
    MusicPageAction, MusicPageState, MusicPageView, MusicProblem, MusicTarget,
};

use super::{Endpoint, Libraries, MusicLink, Page};

const ALBUMS: u32 = MUSIC_PAGE_SIZE + 10;

#[derive(Default)]
struct Library {
    account: AtomicBool,
    /// 非 0 时每个请求都回这个状态码（网关替停了的插件回的那种）。
    status: AtomicU16,
    star_fails: AtomicBool,
    /// 搜这个词要等多久才回。
    slow: Mutex<HashMap<String, u64>>,
}

fn ok(body: Value) -> Response {
    let mut response = json!({"subsonic-response": {"status": "ok", "version": "1.16.1"}});
    if let (Some(target), Some(extra)) = (
        response["subsonic-response"].as_object_mut(),
        body.as_object(),
    ) {
        target.extend(extra.clone());
    }
    axum::Json(response).into_response()
}

fn failed(code: i64) -> Response {
    axum::Json(json!({"subsonic-response": {"status": "failed", "error": {"code": code, "message": "no"}}}))
        .into_response()
}

fn album(n: u32) -> Value {
    json!({"id": format!("al-{n}"), "name": format!("Album {n}"), "artist": "Band", "artistId": "ar-1", "coverArt": format!("al-{n}")})
}

async fn rest(
    State(library): State<Arc<Library>>,
    Path(method): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let status = library.status.load(Ordering::SeqCst);
    if status != 0 {
        return axum::http::StatusCode::from_u16(status)
            .unwrap()
            .into_response();
    }
    if !library.account.load(Ordering::SeqCst) {
        return failed(40);
    }
    let number = |key: &str| {
        query
            .get(key)
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0)
    };
    match method.as_str() {
        "getAlbumList2" => {
            let (offset, size) = (number("offset"), number("size"));
            let albums: Vec<Value> = (offset..ALBUMS.min(offset + size)).map(album).collect();
            ok(json!({"albumList2": {"album": albums}}))
        }
        "getPlaylists" => ok(json!({"playlists": {}})),
        "search3" => {
            let text = query.get("query").cloned().unwrap_or_default();
            let wait = library
                .slow
                .lock()
                .unwrap()
                .get(&text)
                .copied()
                .unwrap_or(0);
            tokio::time::sleep(Duration::from_millis(wait)).await;
            ok(
                json!({"searchResult3": {"song": [{"id": format!("s-{text}"), "title": text, "duration": 3}]}}),
            )
        }
        "getAlbum" => ok(json!({"album": {
            "id": query["id"], "name": "Album", "artist": "Band", "songCount": 3, "duration": 9,
            "song": [
                {"id": "t2", "title": "Two", "artist": "Band", "track": 2, "discNumber": 1, "duration": 3},
                {"id": "t1", "title": "One", "artist": "Guest", "track": 1, "discNumber": 1, "duration": 3},
                {"id": "t3", "title": "Three", "artist": "Band", "track": 1, "discNumber": 2, "duration": 3},
            ],
            "discTitles": [{"disc": 2, "title": "Live"}],
        }})),
        "star" | "unstar" if library.star_fails.load(Ordering::SeqCst) => failed(50),
        "star" | "unstar" => ok(json!({})),
        _ => failed(70),
    }
}

async fn app(State(library): State<Arc<Library>>) -> &'static str {
    library.account.store(true, Ordering::SeqCst);
    "<html></html>"
}

struct Fixed {
    base: Url,
    running: AtomicBool,
}

impl Libraries for Fixed {
    fn endpoint(&self, _: &str) -> Result<Endpoint, MusicProblem> {
        Ok(Endpoint {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            base: self.base.clone(),
        })
    }

    fn running(&self, _: &str) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

struct Harness {
    library: Arc<Library>,
    libraries: Arc<Fixed>,
    link: Arc<MusicLink>,
    _cache: tempfile::TempDir,
}

async fn harness() -> Harness {
    let library = Arc::new(Library::default());
    let router = axum::Router::new()
        .route("/rest/{method}", get(rest))
        .route("/app/", get(app))
        .with_state(library.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let libraries = Arc::new(Fixed {
        base,
        running: AtomicBool::new(true),
    });
    let cache = tempfile::tempdir().unwrap();
    let link = MusicLink::new(
        tokio::runtime::Handle::current(),
        libraries.clone(),
        cache.path().to_owned(),
    );
    Harness {
        library,
        libraries,
        link,
        _cache: cache,
    }
}

async fn until(page: &Page, done: impl Fn(&MusicPageView) -> bool) -> MusicPageView {
    let mut updates = page.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = updates.borrow_and_update().clone();
            if done(&view) {
                return view;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("page never got there: {:?}", page.view()))
}

fn ready(view: &MusicPageView) -> bool {
    matches!(
        view.state,
        MusicPageState::Ready {
            loading_more: false,
            ..
        }
    )
}

fn content(view: &MusicPageView) -> &MusicContent {
    match &view.state {
        MusicPageState::Ready { content, .. } => content,
        state => panic!("not ready: {state:?}"),
    }
}

/// 第一次进音乐页的人在曲库里还没有账号：核心替他打开一次网页建号，页面直接出来，不让他看到“没登录”。
#[tokio::test]
async fn a_person_without_an_account_gets_one_and_the_page_loads() {
    let h = harness().await;
    let page = h.link.open(
        "music".into(),
        MusicPage::Albums {
            sort: AlbumSort::Newest,
        },
    );
    let view = until(&page, ready).await;
    assert!(h.library.account.load(Ordering::SeqCst));
    let MusicContent::Albums { albums } = content(&view) else {
        panic!()
    };
    assert_eq!(albums.len(), MUSIC_PAGE_SIZE as usize);
}

/// 翻页接在后面；最后一页不满就没有下一页了。
#[tokio::test]
async fn loading_more_appends_until_the_library_runs_out() {
    let h = harness().await;
    let page = h.link.open(
        "music".into(),
        MusicPage::Albums {
            sort: AlbumSort::Name,
        },
    );
    until(&page, ready).await;
    h.link.act(&page, MusicPageAction::LoadMore);
    let view = until(&page, |view| {
        matches!(
            &view.state,
            MusicPageState::Ready {
                has_more: false,
                loading_more: false,
                ..
            }
        )
    })
    .await;
    let MusicContent::Albums { albums } = content(&view) else {
        panic!()
    };
    assert_eq!(albums.len(), ALBUMS as usize);
    assert_eq!(albums.last().unwrap().id, format!("al-{}", ALBUMS - 1));
}

/// 插件停了：网关回 502 时说“插件没在跑”，不说曲库出错；已经显示的内容刷新失败时留着，只挂提示。
#[tokio::test]
async fn failures_say_why_and_keep_what_is_already_shown() {
    let h = harness().await;
    h.library.status.store(502, Ordering::SeqCst);
    h.libraries.running.store(false, Ordering::SeqCst);
    let page = h.link.open("music".into(), MusicPage::Home);
    let view = until(&page, |view| {
        matches!(view.state, MusicPageState::Failed { .. })
    })
    .await;
    let MusicPageState::Failed { problem } = view.state else {
        panic!()
    };
    assert_eq!(problem.failure, MusicFailure::PluginNotReady);

    h.library.status.store(0, Ordering::SeqCst);
    h.libraries.running.store(true, Ordering::SeqCst);
    h.link.act(&page, MusicPageAction::Reload);
    until(&page, ready).await;

    h.library.status.store(500, Ordering::SeqCst);
    h.link.act(&page, MusicPageAction::Reload);
    let view = until(&page, |view| {
        matches!(
            &view.state,
            MusicPageState::Ready {
                problem: Some(_),
                ..
            }
        )
    })
    .await;
    let MusicPageState::Ready {
        problem: Some(problem),
        content,
        ..
    } = view.state
    else {
        panic!()
    };
    assert_eq!(problem.failure, MusicFailure::Upstream);
    assert!(matches!(content, MusicContent::Home { .. }));
}

/// 边打边搜：先发出去的慢请求晚回来，也不能盖掉后打的词的结果。
#[tokio::test]
async fn a_slow_answer_for_an_old_query_never_replaces_a_newer_one() {
    let h = harness().await;
    h.library.slow.lock().unwrap().insert("old".into(), 1500);
    h.library.account.store(true, Ordering::SeqCst);
    let page = h.link.open("music".into(), MusicPage::Search);
    until(&page, ready).await;
    h.link
        .act(&page, MusicPageAction::Query { text: "old".into() });
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.link
        .act(&page, MusicPageAction::Query { text: "new".into() });
    until(&page, |view| {
        matches!(&view.state, MusicPageState::Ready { content: MusicContent::Search { query, .. }, .. } if query == "new")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let MusicContent::Search { query, .. } = content(&page.view()).clone() else {
        panic!()
    };
    assert_eq!(query, "new");
}

/// 专辑页按碟分段、碟内按曲号；和专辑歌手相同的不重复写。
#[tokio::test]
async fn an_album_is_split_by_disc_in_track_order() {
    let h = harness().await;
    let page = h
        .link
        .open("music".into(), MusicPage::Album { id: "al-1".into() });
    let view = until(&page, ready).await;
    let MusicContent::Album { album } = content(&view) else {
        panic!()
    };
    let discs: Vec<(Option<u32>, Option<&str>, Vec<&str>)> = album
        .discs
        .iter()
        .map(|disc| {
            let ids = disc.tracks.iter().map(|t| t.id.as_str()).collect();
            (disc.disc, disc.title.as_deref(), ids)
        })
        .collect();
    assert_eq!(
        discs,
        vec![
            (Some(1), None, vec!["t1", "t2"]),
            (Some(2), Some("Live"), vec!["t3"])
        ]
    );
    let artists: Vec<_> = album.discs[0]
        .tracks
        .iter()
        .map(|t| t.artist.as_deref())
        .collect();
    assert_eq!(artists, vec![Some("Guest"), None]);
}

/// 加星先在所有开着的页上显示；曲库拒了就改回来，并把原因交给壳。
#[tokio::test]
async fn a_refused_star_is_rolled_back_on_every_open_page() {
    let h = harness().await;
    let list = h.link.open(
        "music".into(),
        MusicPage::Albums {
            sort: AlbumSort::Newest,
        },
    );
    let home = h.link.open("music".into(), MusicPage::Home);
    until(&list, ready).await;
    until(&home, ready).await;
    let starred = |view: &MusicPageView| match content(view) {
        MusicContent::Albums { albums } => albums[0].starred,
        MusicContent::Home { home } => home.recently_added[0].starred,
        other => panic!("{other:?}"),
    };
    let star = MusicAction::Star {
        target: MusicTarget::Album("al-0".into()),
        starred: true,
    };
    assert_eq!(h.link.star("music", star.clone()).await, None);
    assert!(starred(&list.view()) && starred(&home.view()));

    h.library.star_fails.store(true, Ordering::SeqCst);
    let unstar = MusicAction::Star {
        target: MusicTarget::Album("al-0".into()),
        starred: false,
    };
    let problem = h
        .link
        .star("music", unstar)
        .await
        .expect("the library refused");
    assert_eq!(problem.failure, MusicFailure::PermissionDenied);
    assert!(starred(&list.view()) && starred(&home.view()));
}
