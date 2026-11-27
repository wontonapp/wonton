//! 影视浏览翻视图的行为：图片地址落在本机回环上、带对来源，能放的才带条目，首页某排没取到其余照常。

use wonton_protocol::media::catalog::{
    CatalogEntry, CatalogLibrary, EntryPage, EntryType, Libraries, LibraryKind, Shelf,
};
use wonton_protocol::media::provider::ArtworkKind;
use wonton_protocol::media::{MediaFailure, MediaFailureCode};

use super::{
    Mapper, VideoEntryKind, VideoShelfKind, VideoSort, VideoWallQuery, catalog_path, home,
    wall_query,
};

const PREFIX: &str = "http://127.0.0.1:4100/t0ken";

fn entry(id: &str, entry_type: EntryType, artwork: Vec<ArtworkKind>) -> CatalogEntry {
    serde_json::from_value::<CatalogEntry>(serde_json::json!({
        "id": id, "entry_type": "movie", "title": id,
    }))
    .map(|mut e| {
        e.entry_type = entry_type;
        e.artwork = artwork;
        e
    })
    .unwrap()
}

#[test]
fn pictures_point_at_the_loopback_with_kind_width_and_source() {
    let mapper = Mapper {
        prefix: PREFIX,
        source: Some("app.example.video"),
    };
    let card = mapper.entry(&entry(
        "m/1",
        EntryType::Movie,
        vec![ArtworkKind::Primary, ArtworkKind::Backdrop],
    ));
    assert_eq!(
        card.poster.as_deref(),
        Some(
            "http://127.0.0.1:4100/t0ken/items/m%2F1/artwork?kind=video&source=app.example.video&type=primary&width=480"
        )
    );
    assert!(card.backdrop.unwrap().contains("type=backdrop&width=1600"));
    // 没有缩略图的电影，横版用背景图；没有标志就不给。
    assert!(card.landscape.unwrap().contains("type=backdrop"));
    assert!(card.logo.is_none());
    assert_eq!(card.source.as_deref(), Some("app.example.video"));
    // 起播、下载用的条目也带着同一个来源。
    assert_eq!(
        card.item.unwrap().source.as_deref(),
        Some("app.example.video")
    );
}

#[test]
fn an_episode_without_a_thumb_uses_its_still_as_the_landscape_picture() {
    let mapper = Mapper {
        prefix: PREFIX,
        source: None,
    };
    let card = mapper.entry(&entry("e1", EntryType::Episode, vec![ArtworkKind::Primary]));
    assert!(card.landscape.unwrap().contains("type=primary&width=800"));
    assert!(!card.poster.unwrap().contains("source="));
}

#[test]
fn only_playable_entries_carry_an_item_and_progress_shows_only_mid_way() {
    let mapper = Mapper {
        prefix: PREFIX,
        source: None,
    };
    let series = mapper.entry(&entry("s1", EntryType::Series, Vec::new()));
    assert_eq!(series.kind, VideoEntryKind::Series);
    assert!(series.item.is_none());

    let mut half = entry("m1", EntryType::Movie, Vec::new());
    half.duration_ms = Some(10_000);
    half.position_ms = Some(2_500);
    assert_eq!(mapper.entry(&half).progress, Some(0.25));
    half.played = true;
    assert_eq!(mapper.entry(&half).progress, None);

    let odd = mapper.entry(&entry("x", EntryType::Other("trailer".into()), Vec::new()));
    assert_eq!(odd.kind, VideoEntryKind::Other);
    assert!(odd.item.is_none());
}

#[test]
fn a_missing_shelf_leaves_the_rest_of_home_and_says_why() {
    let mapper = Mapper {
        prefix: PREFIX,
        source: None,
    };
    let libraries = Libraries {
        libraries: vec![CatalogLibrary {
            id: "lib1".into(),
            title: "电影".into(),
            kind: LibraryKind::Movies,
            count: Some(3),
        }],
    };
    let gone = MediaFailure {
        code: MediaFailureCode::UpstreamError,
        message: "500".into(),
        retry_after_secs: None,
    };
    let page = |entries| EntryPage {
        entries,
        cursor: None,
        total: None,
    };
    let view = home(
        &mapper,
        &libraries,
        vec![
            (Shelf::Resume, Err(gone.clone())),
            (Shelf::NextUp, Ok(page(Vec::new()))),
            (
                Shelf::Latest,
                Ok(page(vec![entry("m1", EntryType::Movie, Vec::new())])),
            ),
        ],
    );
    assert_eq!(view.libraries.len(), 1);
    // 空排不给，没取到的那排的原因留给壳出一条轻提示。
    assert_eq!(view.shelves.len(), 1);
    assert_eq!(view.shelves[0].kind, VideoShelfKind::Latest);
    assert_eq!(view.partial_failure, Some(gone));
}

#[test]
fn wall_queries_reach_the_core_with_the_chosen_source() {
    let query = VideoWallQuery {
        library: Some("lib 1".into()),
        kind: Some(VideoEntryKind::Movie),
        search: Some("  ".into()),
        sort: VideoSort::Added,
        unplayed_only: true,
        source: Some("app.example.video".into()),
        ..VideoWallQuery::default()
    };
    let path = catalog_path(&wall_query(&query).path(), query.source.as_deref());
    assert_eq!(
        path,
        "/_wonton/api/media/catalog/entries?library=lib%201&entry_type=movie&sort=added&unplayed=true&source=app.example.video"
    );
    assert_eq!(
        catalog_path("/catalog/libraries", None),
        "/_wonton/api/media/catalog/libraries"
    );
}
