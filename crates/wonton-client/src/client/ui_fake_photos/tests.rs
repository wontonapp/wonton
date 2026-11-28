use wonton_protocol::media::MediaFailureCode;
use wonton_protocol::media::library_view::{PhotoEdit, PhotoEditKind, PhotosAction};

use super::{FakeMode, actions::apply, opening, picture};

fn timeline_ids(view: &wonton_protocol::media::library_view::PhotosViewModel) -> Vec<String> {
    view.timeline
        .months
        .iter()
        .flat_map(|month| &month.days)
        .flat_map(|day| &day.tiles)
        .map(|tile| tile.id.clone())
        .collect()
}

#[test]
fn trashing_a_mixed_selection_reports_the_item_that_stayed() {
    let mut view = opening(FakeMode::Library);
    let ids = vec!["fake-a".to_owned(), "fake-from-mom".to_owned()];
    apply(
        &mut view,
        PhotosAction::Select {
            ids: ids.clone(),
            selected: true,
        },
        FakeMode::Library,
    );
    apply(
        &mut view,
        PhotosAction::Edit {
            edit: PhotoEdit::Trash { ids },
        },
        FakeMode::Library,
    );

    let left = timeline_ids(&view);
    assert!(!left.contains(&"fake-a".to_owned()));
    assert!(left.contains(&"fake-from-mom".to_owned()));
    assert_eq!(view.selected, ["fake-from-mom"]);
    let outcome = view.last_edit.expect("the edit leaves an outcome");
    assert_eq!(outcome.kind, PhotoEditKind::Trash);
    assert_eq!(outcome.done, ["fake-a"]);
    assert_eq!(outcome.failed.len(), 1);
    assert_eq!(
        outcome.failed[0].failure.code,
        MediaFailureCode::PermissionDenied
    );
}

#[test]
fn a_library_that_needs_a_source_opens_once_one_is_picked() {
    let mode = FakeMode::SourceRequired;
    let mut view = opening(mode);
    let open = |source: Option<&str>| PhotosAction::Open {
        utc_offset_minutes: 480,
        source: source.map(str::to_owned),
    };
    apply(&mut view, open(None), mode);
    assert_eq!(
        view.timeline.problem.as_ref().map(|problem| &problem.code),
        Some(&MediaFailureCode::SourceRequired)
    );
    assert_eq!(view.actions, None);

    apply(&mut view, open(Some("immich")), mode);
    assert_eq!(view.timeline.problem, None);
    assert!(!timeline_ids(&view).is_empty());
}

#[test]
fn two_photo_apps_ask_first_and_open_the_one_picked() {
    use wonton_protocol::media::library_view::PhotoSourceState;

    let mode = FakeMode::ChooseSource;
    let mut view = opening(mode);
    assert_eq!(view.sources.state, PhotoSourceState::Choose);
    assert_eq!(view.sources.candidates.len(), 2);
    assert!(view.timeline.months.is_empty());

    let picked = view.sources.candidates[1].id.clone();
    apply(
        &mut view,
        PhotosAction::ChooseSource { id: picked.clone() },
        mode,
    );
    assert_eq!(view.sources.state, PhotoSourceState::Ready);
    assert_eq!(view.sources.chosen, Some(picked));
    assert!(!timeline_ids(&view).is_empty());
}

#[test]
fn a_month_loads_in_two_pages_and_stops_at_its_count() {
    let mut view = opening(FakeMode::Library);
    let month = || "2026-10".to_owned();
    let loaded = |view: &wonton_protocol::media::library_view::PhotosViewModel| {
        let october = &view.timeline.months[1];
        let tiles: usize = october.days.iter().map(|day| day.tiles.len()).sum();
        (tiles, october.has_more)
    };
    apply(
        &mut view,
        PhotosAction::OpenMonth { month: month() },
        FakeMode::Library,
    );
    let (first, more) = loaded(&view);
    assert!(first < 120 && more);

    apply(
        &mut view,
        PhotosAction::LoadMoreMonth { month: month() },
        FakeMode::Library,
    );
    apply(
        &mut view,
        PhotosAction::LoadMoreMonth { month: month() },
        FakeMode::Library,
    );
    assert_eq!(loaded(&view), (120, false));
    let ids = timeline_ids(&view);
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn a_picture_keeps_the_requested_shape() {
    let wide = picture::bmp("fake-a", 240, 4.0 / 3.0);
    assert_eq!(&wide[..2], b"BM");
    let size = |bytes: &[u8]| {
        let field = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        (field(18), field(22))
    };
    assert_eq!(size(&wide), (240, 180));
    assert_eq!(wide.len(), 54 + 240 * 180 * 3);
    assert_eq!(
        size(&picture::bmp("fake-tall", 240, 9.0 / 16.0)),
        (135, 240)
    );
}
