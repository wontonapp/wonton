//! 线上的字段翻成壳要的视图。

use wonton_protocol::music::{
    AlbumDetail, AlbumTile, ArtistDetail, ArtistRow, DiscSection, GenreRow, LyricLine, MusicLyrics,
    PlaylistDetail, PlaylistRow, TrackRow,
};

use super::wire;

fn ms(seconds: u64) -> u64 {
    seconds.saturating_mul(1000)
}

pub(super) fn album(album: &wire::Album) -> AlbumTile {
    AlbumTile {
        id: album.id.clone(),
        title: album.name.clone(),
        artist: album.artist.clone(),
        artist_id: album.artist_id.clone(),
        year: album.year.filter(|year| *year > 0),
        cover: album.cover_art.clone(),
        starred: album.starred.is_some(),
    }
}

pub(super) fn albums(albums: &[wire::Album]) -> Vec<AlbumTile> {
    albums.iter().map(album).collect()
}

pub(super) fn track(song: &wire::Song) -> TrackRow {
    TrackRow {
        id: song.id.clone(),
        title: song.title.clone(),
        artist: song.artist.clone(),
        artist_id: song.artist_id.clone(),
        album: song.album.clone(),
        album_id: song.album_id.clone(),
        track: song.track.filter(|n| *n > 0),
        disc: song.disc_number.filter(|n| *n > 0),
        duration_ms: ms(song.duration),
        cover: song.cover_art.clone(),
        starred: song.starred.is_some(),
    }
}

pub(super) fn tracks(songs: &[wire::Song]) -> Vec<TrackRow> {
    songs.iter().map(track).collect()
}

pub(super) fn artist(artist: &wire::Artist) -> ArtistRow {
    ArtistRow {
        id: artist.id.clone(),
        name: artist.name.clone(),
        album_count: artist.album_count,
        cover: artist.cover_art.clone(),
        starred: artist.starred.is_some(),
    }
}

pub(super) fn artists(artists: &[wire::Artist]) -> Vec<ArtistRow> {
    artists.iter().map(artist).collect()
}

/// 自己改不了的歌单（OpenSubsonic 的 `readonly`）才写归属人；上游没给这个字段时写上，宁可多一行字也不把别人的歌单当成自己的。
pub(super) fn playlist(playlist: &wire::Playlist) -> PlaylistRow {
    PlaylistRow {
        id: playlist.id.clone(),
        name: playlist.name.clone(),
        owner: playlist
            .owner
            .clone()
            .filter(|owner| !owner.is_empty() && playlist.readonly != Some(false)),
        song_count: playlist.song_count,
        duration_ms: ms(playlist.duration),
        cover: playlist.cover_art.clone(),
    }
}

pub(super) fn genre(genre: &wire::Genre) -> GenreRow {
    GenreRow {
        name: genre.value.clone(),
        album_count: genre.album_count,
        song_count: genre.song_count,
    }
}

/// 专辑页：按碟分段，碟内按曲号；和专辑歌手相同的歌手名去掉，壳只在合辑里显示第二行。
pub(super) fn album_detail(album: &wire::Album) -> AlbumDetail {
    let mut songs: Vec<&wire::Song> = album.songs.iter().collect();
    songs.sort_by_key(|song| {
        (
            song.disc_number.unwrap_or(1),
            song.track.unwrap_or(u32::MAX),
        )
    });
    let multi_disc = songs
        .iter()
        .any(|song| song.disc_number.unwrap_or(1) != songs[0].disc_number.unwrap_or(1));
    let mut discs: Vec<DiscSection> = Vec::new();
    for song in songs {
        let number = song.disc_number.unwrap_or(1);
        let mut row = track(song);
        if row.artist == album.artist {
            row.artist = None;
            row.artist_id = None;
        }
        match discs.last_mut() {
            Some(section) if !multi_disc || section.disc == Some(number) => {
                section.tracks.push(row)
            }
            _ => discs.push(DiscSection {
                disc: multi_disc.then_some(number),
                title: multi_disc
                    .then(|| {
                        album
                            .disc_titles
                            .iter()
                            .find(|disc| disc.disc == number && !disc.title.is_empty())
                            .map(|disc| disc.title.clone())
                    })
                    .flatten(),
                tracks: vec![row],
            }),
        }
    }
    AlbumDetail {
        album: self::album(album),
        genre: album.genre.clone().filter(|genre| !genre.is_empty()),
        song_count: album.song_count,
        duration_ms: ms(album.duration),
        discs,
    }
}

/// 歌手页：专辑从新到旧，没有年份的排最后。
pub(super) fn artist_detail(artist: &wire::Artist) -> ArtistDetail {
    let mut albums = albums(&artist.albums);
    albums.sort_by_key(|album| std::cmp::Reverse(album.year.unwrap_or(0)));
    ArtistDetail {
        artist: self::artist(artist),
        albums,
    }
}

pub(super) fn playlist_detail(playlist: &wire::Playlist) -> PlaylistDetail {
    PlaylistDetail {
        playlist: self::playlist(playlist),
        comment: playlist
            .comment
            .clone()
            .filter(|comment| !comment.is_empty()),
        tracks: tracks(&playlist.entries),
    }
}

/// 有同步的取同步那份，没有就取第一份；一份都没有、或者全是空行时为空。
pub(super) fn lyrics(list: &wire::LyricsList) -> Option<MusicLyrics> {
    let chosen = list
        .structured_lyrics
        .iter()
        .find(|lyrics| lyrics.synced)
        .or_else(|| list.structured_lyrics.first())?;
    if chosen.line.iter().all(|line| line.value.trim().is_empty()) {
        return None;
    }
    Some(MusicLyrics {
        synced: chosen.synced,
        lines: chosen
            .line
            .iter()
            .map(|line| LyricLine {
                start_ms: if chosen.synced { line.start } else { None },
                text: line.value.clone(),
            })
            .collect(),
    })
}
