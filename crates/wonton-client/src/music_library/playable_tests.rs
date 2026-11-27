//! 点一首歌时要放的那一列：页面顺序、补回的署名、来源。

use wonton_protocol::music::{
    AlbumDetail, AlbumTile, DiscSection, MusicContent, MusicHome, MusicPage, MusicPageState,
    MusicPageView, PlaylistDetail, PlaylistRow, TrackRow,
};

use super::playable;

const PLUGIN: &str = "navidrome";

fn track(id: &str, artist: Option<&str>) -> TrackRow {
    TrackRow {
        id: id.into(),
        title: format!("歌 {id}"),
        artist: artist.map(Into::into),
        artist_id: None,
        album: Some("海边".into()),
        album_id: Some("al1".into()),
        track: Some(1),
        disc: None,
        duration_ms: 180_000,
        cover: None,
        starred: false,
    }
}

fn ready(page: MusicPage, content: MusicContent) -> MusicPageView {
    MusicPageView {
        page,
        state: MusicPageState::Ready {
            content,
            has_more: false,
            loading_more: false,
            problem: None,
        },
    }
}

fn ids(view: &MusicPageView) -> Vec<String> {
    playable(view, PLUGIN)
        .into_iter()
        .map(|item| item.id)
        .collect()
}

#[test]
fn an_album_plays_every_disc_in_order_and_puts_the_album_artist_back() {
    let album = AlbumDetail {
        album: AlbumTile {
            id: "al1".into(),
            title: "海边".into(),
            artist: Some("阿海".into()),
            artist_id: None,
            year: None,
            cover: None,
            starred: false,
        },
        genre: None,
        song_count: 3,
        duration_ms: 540_000,
        discs: vec![
            DiscSection {
                disc: Some(1),
                title: None,
                tracks: vec![track("a", None), track("b", Some("客串"))],
            },
            DiscSection {
                disc: Some(2),
                title: None,
                tracks: vec![track("c", None)],
            },
        ],
    };
    let view = ready(
        MusicPage::Album { id: "al1".into() },
        MusicContent::Album { album },
    );
    let items = playable(&view, PLUGIN);
    let artists: Vec<_> = items.iter().map(|item| item.artist.as_deref()).collect();
    assert_eq!(ids(&view), ["a", "b", "c"]);
    assert_eq!(artists, [Some("阿海"), Some("客串"), Some("阿海")]);
    assert!(
        items
            .iter()
            .all(|item| item.source.as_deref() == Some(PLUGIN))
    );
}

#[test]
fn a_playlist_keeps_a_song_that_appears_twice() {
    let playlist = PlaylistDetail {
        playlist: PlaylistRow {
            id: "p1".into(),
            name: "通勤".into(),
            owner: None,
            song_count: 3,
            duration_ms: 540_000,
            cover: None,
        },
        comment: None,
        tracks: vec![track("a", None), track("b", None), track("a", None)],
    };
    let view = ready(
        MusicPage::Playlist { id: "p1".into() },
        MusicContent::Playlist { playlist },
    );
    assert_eq!(ids(&view), ["a", "b", "a"]);
}

#[test]
fn pages_without_songs_and_pages_still_loading_have_nothing_to_play() {
    let home = ready(
        MusicPage::Home,
        MusicContent::Home {
            home: MusicHome {
                recently_added: Vec::new(),
                recently_played: Vec::new(),
                most_played: Vec::new(),
                playlists: Vec::new(),
            },
        },
    );
    assert!(ids(&home).is_empty());
    let loading = MusicPageView {
        page: MusicPage::Songs,
        state: MusicPageState::Loading,
    };
    assert!(ids(&loading).is_empty());
}
