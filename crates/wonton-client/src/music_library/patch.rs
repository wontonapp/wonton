//! 对已有视图的改动：请求落地、翻页接上、加星。

use wonton_protocol::music::{
    AlbumTile, ArtistRow, MusicContent, MusicPageState, MusicPageView, MusicProblem, MusicTarget,
    TrackRow,
};

use super::fetch::Loaded;

/// 已经取到多少条（翻页的偏移）。不翻页的页是 0。
pub(super) fn loaded_len(state: &MusicPageState) -> u32 {
    let len = match state {
        MusicPageState::Ready {
            content: MusicContent::Albums { albums },
            ..
        } => albums.len(),
        MusicPageState::Ready {
            content: MusicContent::Songs { songs },
            ..
        } => songs.len(),
        _ => 0,
    };
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// 一次请求落地。成功：换掉或接上内容；失败：有内容时挂轻提示，没有时整页出错。
pub(super) fn land(state: &mut MusicPageState, append: bool, result: Result<Loaded, MusicProblem>) {
    match result {
        Ok(loaded) if append => {
            let MusicPageState::Ready {
                content,
                has_more,
                loading_more,
                problem,
            } = state
            else {
                return;
            };
            match (content, loaded.content) {
                (MusicContent::Albums { albums }, MusicContent::Albums { albums: more }) => {
                    albums.extend(more);
                }
                (MusicContent::Songs { songs }, MusicContent::Songs { songs: more }) => {
                    songs.extend(more);
                }
                _ => {}
            }
            *has_more = loaded.has_more;
            *loading_more = false;
            *problem = None;
        }
        Ok(loaded) => {
            *state = MusicPageState::Ready {
                content: loaded.content,
                has_more: loaded.has_more,
                loading_more: false,
                problem: None,
            };
        }
        Err(failure) => {
            if let MusicPageState::Ready {
                loading_more,
                problem,
                ..
            } = state
            {
                *loading_more = false;
                *problem = Some(failure);
            } else {
                *state = MusicPageState::Failed { problem: failure };
            }
        }
    }
}

/// 把页里所有指向 `target` 的条目标上或去掉星。
pub(super) fn star(view: &mut MusicPageView, target: &MusicTarget, starred: bool) {
    let MusicPageState::Ready { content, .. } = &mut view.state else {
        return;
    };
    let songs = |rows: &mut [TrackRow]| {
        if let MusicTarget::Song(id) = target {
            rows.iter_mut()
                .filter(|row| &row.id == id)
                .for_each(|row| row.starred = starred);
        }
    };
    let albums = |tiles: &mut [AlbumTile]| {
        if let MusicTarget::Album(id) = target {
            tiles
                .iter_mut()
                .filter(|tile| &tile.id == id)
                .for_each(|tile| tile.starred = starred);
        }
    };
    let artists = |rows: &mut [ArtistRow]| {
        if let MusicTarget::Artist(id) = target {
            rows.iter_mut()
                .filter(|row| &row.id == id)
                .for_each(|row| row.starred = starred);
        }
    };
    match content {
        MusicContent::Home { home } => {
            albums(&mut home.recently_added);
            albums(&mut home.recently_played);
            albums(&mut home.most_played);
        }
        MusicContent::Albums { albums: tiles } => albums(tiles),
        MusicContent::Artists { artists: rows } => artists(rows),
        MusicContent::Songs { songs: rows } => songs(rows),
        MusicContent::Starred { starred: found } => {
            artists(&mut found.artists);
            albums(&mut found.albums);
            songs(&mut found.songs);
        }
        MusicContent::Search { results, .. } => {
            artists(&mut results.artists);
            albums(&mut results.albums);
            songs(&mut results.songs);
        }
        MusicContent::Album { album } => {
            albums(std::slice::from_mut(&mut album.album));
            album
                .discs
                .iter_mut()
                .for_each(|disc| songs(&mut disc.tracks));
        }
        MusicContent::Artist { artist } => {
            artists(std::slice::from_mut(&mut artist.artist));
            albums(&mut artist.albums);
        }
        MusicContent::Playlist { playlist } => songs(&mut playlist.tracks),
        MusicContent::Playlists { .. } | MusicContent::Genres { .. } => {}
    }
}
