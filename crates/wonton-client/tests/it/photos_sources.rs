//! 用哪个相册插件、能共享给谁：只经导出给原生壳的接口驱动，假核心回 `/sources`、`/share-targets` 的名单，
//! 断言看 ViewModel 和假核心收到的请求。

use std::time::Duration;

use axum::http::Method;
use wonton_protocol::MEMBERS_PATH;
use wonton_protocol::media::MediaFailureCode;
use wonton_protocol::media::choices::SHARE_TARGETS_PATH;
use wonton_protocol::media::library_view::{PhotoSourceState, PhotosAction, PhotosViewModel};

use crate::photos::{november, open, photos, send};
use crate::photos_support::{PHOTOS_SOURCE, envelope, source, target};
use crate::plugins::rig::{Canned, Rig};

const LIBRARY: &str = "/_wonton/api/media/library";
const TIMELINE: &str = "/_wonton/api/media/library/timeline";

fn has_months(view: &PhotosViewModel) -> bool {
    view.sources.state == PhotoSourceState::Ready
        && !view.timeline.loading
        && !view.timeline.months.is_empty()
}

/// 取时间线时带没带 `source`：(不带的次数, 带的次数)。
fn timeline_requests(rig: &Rig) -> (usize, usize) {
    let all = rig.core.all_seen();
    let timeline: Vec<_> = all
        .iter()
        .filter(|seen| seen.method == Method::GET && seen.path.starts_with(TIMELINE))
        .collect();
    let plain = timeline.iter().filter(|seen| seen.path == TIMELINE).count();
    (plain, timeline.len() - plain)
}

fn library_requests(rig: &Rig) -> usize {
    rig.core
        .all_seen()
        .iter()
        .filter(|seen| seen.path.starts_with(LIBRARY))
        .count()
}

#[tokio::test]
async fn a_single_photo_app_is_used_without_asking() {
    let rig = Rig::new().await;
    november(&rig);
    rig.connected().await;
    open(&rig);
    let view = photos(&rig, "时间线", has_months).await;
    let ids: Vec<_> = view
        .sources
        .candidates
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, [PHOTOS_SOURCE]);
    // 不点名：核心照规则用唯一那家。
    assert_eq!(view.sources.chosen, None);
    let (plain, named) = timeline_requests(&rig);
    assert!(plain >= 1 && named == 0, "{plain} {named}");
}

#[tokio::test]
async fn no_photo_app_is_reported_as_not_installed_not_as_a_failure() {
    let rig = Rig::new().await;
    rig.core.photos.sources(Vec::new());
    rig.connected().await;
    open(&rig);
    let view = photos(&rig, "没装", |view| {
        view.sources.state != PhotoSourceState::Checking
    })
    .await;
    assert_eq!(view.sources.state, PhotoSourceState::NotInstalled);
    assert_eq!(view.sources.problem, None);
    assert_eq!(view.timeline.problem, None);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(library_requests(&rig), 0);
}

#[tokio::test]
async fn several_photo_apps_wait_for_a_choice_that_outlives_a_restart() {
    let mut rig = Rig::new().await;
    rig.core
        .photos
        .sources(vec![source("app.a", false), source("app.b", false)]);
    november(&rig);
    rig.connected().await;
    open(&rig);
    let view = photos(&rig, "要挑一家", |view| {
        view.sources.state == PhotoSourceState::Choose
    })
    .await;
    assert_eq!(view.sources.candidates.len(), 2);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(library_requests(&rig), 0);

    send(&rig, PhotosAction::ChooseSource { id: "app.b".into() }).unwrap();
    let view = photos(&rig, "挑好以后的时间线", has_months).await;
    assert_eq!(view.sources.chosen.as_deref(), Some("app.b"));
    let picked = format!("{TIMELINE}?source=app.b");
    let before = rig.core.seen(&Method::GET, &picked).len();
    assert!(before >= 1);
    assert_eq!(timeline_requests(&rig).0, 0);

    // 关掉 App 再打开：不再问，直接用挑过的那家。
    rig.reopen();
    rig.connected().await;
    open(&rig);
    let view = photos(&rig, "重开后的时间线", has_months).await;
    assert_eq!(view.sources.chosen.as_deref(), Some("app.b"));
    assert!(rig.core.seen(&Method::GET, &picked).len() > before);
    assert_eq!(timeline_requests(&rig).0, 0);
}

#[tokio::test]
async fn the_owners_default_is_used_without_asking() {
    let rig = Rig::new().await;
    rig.core
        .photos
        .sources(vec![source("app.a", false), source("app.b", true)]);
    november(&rig);
    rig.connected().await;
    open(&rig);
    let view = photos(&rig, "时间线", has_months).await;
    assert_eq!(view.sources.chosen, None);
    let (plain, named) = timeline_requests(&rig);
    assert!(plain >= 1 && named == 0, "{plain} {named}");
}

#[tokio::test]
async fn a_member_who_cannot_manage_members_still_gets_the_share_list() {
    let rig = Rig::new().await;
    rig.core.family.member_without_managing();
    rig.core
        .photos
        .share_targets(vec![target("owner"), target("mom")]);
    rig.connected().await;
    open(&rig);
    let view = photos(&rig, "能共享给的人", |view| {
        !view.share_targets.loading && !view.share_targets.people.is_empty()
    })
    .await;
    let people: Vec<_> = view
        .share_targets
        .people
        .iter()
        .map(|person| person.user_id.as_str())
        .collect();
    assert_eq!(people, ["@owner:example.com", "@mom:example.com"]);
    assert_eq!(view.share_targets.problem, None);
    assert!(rig.core.seen(&Method::GET, MEMBERS_PATH).is_empty());

    // 核心答不上：如实说，不留旧名单。
    rig.core.answer(
        Method::GET,
        SHARE_TARGETS_PATH,
        Canned(
            503,
            envelope(MediaFailureCode::TransportUnavailable, "down"),
        ),
    );
    send(&rig, PhotosAction::Refresh).unwrap();
    let view = photos(&rig, "名单没取成", |view| {
        !view.share_targets.loading && view.share_targets.problem.is_some()
    })
    .await;
    assert_eq!(
        view.share_targets.problem.unwrap().code,
        MediaFailureCode::TransportUnavailable
    );
    assert!(view.share_targets.people.is_empty());
}
