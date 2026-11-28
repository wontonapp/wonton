use super::super::provider::{ArtworkQuery, item_artwork_path};
use super::super::{MediaFailure, MediaFailureCode, TrackKind};
use super::*;

fn episode() -> CatalogEntry {
    CatalogEntry {
        id: "ep/1".into(),
        entry_type: EntryType::Episode,
        title: "启程".into(),
        year: Some(2024),
        series_id: Some("s1".into()),
        series_title: Some("海边".into()),
        season: Some(1),
        episode: Some(3),
        duration_ms: Some(2_700_000),
        position_ms: Some(600_000),
        played: false,
        favorite: true,
        unplayed_count: None,
        child_count: None,
        community_rating: Some(8.1),
        official_rating: Some("TV-14".into()),
        added_at: Some(1_700_000_000_000),
        artwork: vec![ArtworkKind::Primary, ArtworkKind::Thumb],
    }
}

fn series() -> CatalogEntry {
    CatalogEntry {
        id: "s1".into(),
        entry_type: EntryType::Series,
        title: "海边".into(),
        series_id: None,
        series_title: None,
        season: None,
        episode: None,
        position_ms: None,
        unplayed_count: Some(5),
        child_count: Some(2),
        artwork: vec![],
        ..episode()
    }
}

fn roundtrip<T>(value: &T) -> T
where
    T: Serialize + serde::de::DeserializeOwned,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

#[test]
fn paths_escape_ids_and_carry_only_given_parameters() {
    assert_eq!(entry_path("ep/1"), "/catalog/entries/ep%2F1");
    assert_eq!(shelf_path(&Shelf::NextUp, None), "/catalog/shelves/next_up");
    assert_eq!(
        shelf_path(&Shelf::Resume, Some("lib 1")),
        "/catalog/shelves/resume?library=lib%201"
    );
    assert_eq!(genres_path(None), "/catalog/genres");
    assert_eq!(genres_path(Some("m")), "/catalog/genres?library=m");
    // 演职员照片和背景图都走契约里的封面路由。
    let backdrop = ArtworkQuery {
        artwork: Some(ArtworkKind::Backdrop),
        width: Some(1280),
    };
    assert_eq!(
        item_artwork_path("p/9", &backdrop),
        "/items/p%2F9/artwork?type=backdrop&width=1280"
    );
    for path in [
        PATH_CATALOG_LIBRARIES,
        PATH_CATALOG_ENTRIES,
        PATH_CATALOG_ACTIONS,
    ] {
        assert!(path.starts_with(PATH_CATALOG));
    }
}

#[test]
fn entries_query_builds_the_path_from_set_fields() {
    assert_eq!(EntriesQuery::default().path(), "/catalog/entries");
    let query = EntriesQuery {
        library: Some("movies".into()),
        genre: Some("科幻 & 冒险".into()),
        sort: Some(SortBy::Added),
        unplayed: Some(true),
        ..EntriesQuery::default()
    };
    assert_eq!(
        query.path(),
        "/catalog/entries?library=movies&genre=%E7%A7%91%E5%B9%BB%20%26%20%E5%86%92%E9%99%A9&sort=added&unplayed=true"
    );
    let json = serde_json::to_value(&query).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"library": "movies", "genre": "科幻 & 冒险", "sort": "added", "unplayed": true})
    );
    assert_eq!(roundtrip(&query), query);
}

#[test]
fn a_bare_entry_reads_with_defaults_and_omits_empty_fields() {
    let bare: CatalogEntry = serde_json::from_value(serde_json::json!({
        "id": "m1", "entry_type": "movie", "title": "长夜"
    }))
    .unwrap();
    assert!(!bare.played && !bare.favorite && bare.artwork.is_empty());
    let json = serde_json::to_value(&bare).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"id": "m1", "entry_type": "movie", "title": "长夜", "played": false, "favorite": false})
    );
    let page = EntryPage {
        entries: vec![episode(), series()],
        cursor: Some("50".into()),
        total: Some(120),
    };
    assert_eq!(roundtrip(&page), page);
}

#[test]
fn only_playable_entries_turn_into_items_for_playback_and_download() {
    let item = episode().to_item(Some("app.example.video")).unwrap();
    assert_eq!(item.id, "ep/1");
    assert_eq!(item.kind, MediaKind::Video);
    assert_eq!(item.series.as_deref(), Some("海边"));
    assert_eq!(item.index, Some(3));
    assert_eq!(item.duration_ms, Some(2_700_000));
    assert!(item.starred && !item.played);
    assert_eq!(item.source.as_deref(), Some("app.example.video"));
    assert_eq!(
        item.artwork_url.as_deref(),
        Some("/_wonton/api/media/items/ep%2F1/artwork?kind=video&source=app.example.video")
    );
    let unsourced = episode().to_item(None).unwrap();
    assert!(unsourced.source.is_none());
    assert_eq!(
        unsourced.artwork_url.as_deref(),
        Some("/_wonton/api/media/items/ep%2F1/artwork?kind=video")
    );

    let no_poster = CatalogEntry {
        entry_type: EntryType::Movie,
        artwork: vec![ArtworkKind::Backdrop],
        ..episode()
    };
    assert!(no_poster.to_item(None).unwrap().artwork_url.is_none());

    for entry_type in [
        EntryType::Series,
        EntryType::Season,
        EntryType::Folder,
        EntryType::Other("trailer".into()),
    ] {
        let entry = CatalogEntry {
            entry_type,
            ..series()
        };
        assert!(entry.to_item(Some("app.example.video")).is_none());
    }
}

#[test]
fn unknown_values_from_the_server_fall_to_other_and_roundtrip() {
    let kind: LibraryKind = serde_json::from_str("\"concerts\"").unwrap();
    assert_eq!(kind, LibraryKind::Other("concerts".into()));
    assert_eq!(serde_json::to_string(&kind).unwrap(), "\"concerts\"");

    let mut json = serde_json::to_value(episode()).unwrap();
    json["entry_type"] = serde_json::json!("trailer");
    json["artwork"] = serde_json::json!(["primary", "disc"]);
    let parsed: CatalogEntry = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(parsed.entry_type, EntryType::Other("trailer".into()));
    assert_eq!(parsed.artwork[1], ArtworkKind::Other("disc".into()));
    assert_eq!(serde_json::to_value(&parsed).unwrap(), json);

    let role: CreditRole = serde_json::from_str("\"composer\"").unwrap();
    assert_eq!(role, CreditRole::Other("composer".into()));
    let shelf: Shelf = serde_json::from_str("\"trending\"").unwrap();
    assert_eq!(shelf.as_str(), "trending");
    let sort: SortBy = serde_json::from_str("\"runtime\"").unwrap();
    assert_eq!(sort.as_str(), "runtime");
    assert_eq!(SortBy::default(), SortBy::Title);
}

#[test]
fn detail_carries_children_next_up_and_tracks() {
    let detail = EntryDetail {
        entry: series(),
        overview: Some("一家人在海边过了一年。".into()),
        tagline: None,
        genres: vec!["剧情".into()],
        studios: vec![],
        premiered_at: Some(1_704_067_200_000),
        people: vec![Credit {
            id: Some("p1".into()),
            name: "某演员".into(),
            role: CreditRole::Actor,
            character: Some("姐姐".into()),
            artwork: true,
        }],
        children: vec![CatalogEntry {
            id: "season-1".into(),
            entry_type: EntryType::Season,
            title: "第 1 季".into(),
            season: Some(1),
            ..series()
        }],
        next_up: Some(episode()),
        audio: vec![MediaTrack {
            index: 1,
            language: "zho".into(),
            label: Some("国语 5.1".into()),
            kind: TrackKind::Audio,
            selected: true,
        }],
        subtitles: vec![],
        media: Some(MediaInfo {
            width: Some(3840),
            height: Some(2160),
            hdr: true,
            video_codec: Some("hevc".into()),
            audio_codec: Some("eac3".into()),
            audio_channels: Some(6),
            container: Some("mkv".into()),
            size: Some(12_000_000_000),
        }),
    };
    assert_eq!(roundtrip(&detail), detail);

    let bare: EntryDetail = serde_json::from_value(serde_json::json!({
        "entry": {"id": "m1", "entry_type": "movie", "title": "长夜"}
    }))
    .unwrap();
    assert!(bare.children.is_empty() && bare.next_up.is_none() && bare.media.is_none());

    let libraries = Libraries {
        libraries: vec![CatalogLibrary {
            id: "m".into(),
            title: "电影".into(),
            kind: LibraryKind::Movies,
            count: Some(312),
        }],
    };
    assert_eq!(roundtrip(&libraries), libraries);
    let genres = Genres {
        genres: vec!["动画".into(), "剧情".into()],
    };
    assert_eq!(roundtrip(&genres), genres);
}

#[test]
fn actions_are_tagged_and_unknown_ones_are_rejected() {
    let action = CatalogAction::SetPlayed {
        ids: vec!["s1".into()],
        played: true,
    };
    let json = serde_json::to_value(&action).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind": "set_played", "ids": ["s1"], "played": true})
    );
    assert_eq!(roundtrip(&action), action);
    let unknown = serde_json::from_value::<CatalogAction>(
        serde_json::json!({"kind": "delete", "ids": ["s1"]}),
    );
    assert!(unknown.is_err());

    let result = CatalogActionResult {
        changed: 1,
        failed: vec![ActionFailure {
            id: "m9".into(),
            failure: MediaFailure {
                code: MediaFailureCode::MediaGone,
                message: "gone".into(),
                retry_after_secs: None,
            },
        }],
    };
    assert_eq!(roundtrip(&result), result);
    let empty: CatalogActionResult =
        serde_json::from_value(serde_json::json!({"changed": 0})).unwrap();
    assert!(empty.failed.is_empty());
}
