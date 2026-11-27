//! 假曲库按 OpenSubsonic 的响应形状回：只盖浏览层用到的端点，参数照上游的意思处理（分页、排序、按流派、搜索、加星）。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Mutex;

use serde_json::{Map, Value, json};

use super::data::{
    ALBUMS, ARTISTS, AlbumDef, FREQUENT, PLAIN_LYRICS, PLAYLISTS, PlaylistDef, RECENT, STARRED,
    SYNCED_LYRICS,
};

/// 收藏的时刻：浏览层只看有没有。
const STARRED_AT: &str = "2026-11-01T10:00:00Z";

/// Subsonic 的“找不到”。
const NOT_FOUND: i64 = 70;

pub(super) enum Reply {
    /// `subsonic-response` 里除了 `status`、`version` 的字段。
    Ok(Map<String, Value>),
    Image(Vec<u8>),
    Failed(i64, &'static str),
}

pub(super) struct Catalog {
    starred: Mutex<HashSet<String>>,
}

type Params = BTreeMap<String, String>;

fn one(key: &str, value: Value) -> Reply {
    let mut map = Map::new();
    map.insert(key.to_owned(), value);
    Reply::Ok(map)
}

fn number(params: &Params, key: &str, default: usize) -> usize {
    params
        .get(key)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn artist_name(id: &str) -> &'static str {
    ARTISTS
        .iter()
        .find(|artist| artist.id == id)
        .map_or("", |artist| artist.name)
}

/// 每首歌的时长按 ID 算，两分多到五分钟。
fn duration(id: &str) -> u64 {
    let hash = id.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    140 + u64::from(hash % 170)
}

/// 一张专辑的歌：ID、碟号、碟内第几首、歌名、署名。
fn songs(album: &AlbumDef) -> Vec<(String, u32, u32, &'static str, &'static str)> {
    let per_disc = if album.disc_titles.is_empty() {
        album.tracks.len()
    } else {
        album.tracks.len().div_ceil(album.disc_titles.len())
    };
    album
        .tracks
        .iter()
        .enumerate()
        .map(|(index, title)| {
            let artist = match album.guest {
                Some((track, name)) if track == index + 1 => name,
                _ => artist_name(album.artist),
            };
            let disc = u32::try_from(index / per_disc + 1).unwrap_or(1);
            let track = u32::try_from(index % per_disc + 1).unwrap_or(1);
            (
                format!("{}-{}", album.id, index + 1),
                disc,
                track,
                *title,
                artist,
            )
        })
        .collect()
}

fn find_song(id: &str) -> Option<(&'static AlbumDef, u32, u32, &'static str, &'static str)> {
    let album = ALBUMS
        .iter()
        .find(|album| id.starts_with(&format!("{}-", album.id)))?;
    songs(album)
        .into_iter()
        .find(|song| song.0 == id)
        .map(|(_, disc, track, title, artist)| (album, disc, track, title, artist))
}

impl Catalog {
    pub(super) fn new() -> Self {
        Self {
            starred: Mutex::new(STARRED.iter().map(|id| (*id).to_owned()).collect()),
        }
    }

    fn is_starred(&self, id: &str) -> bool {
        self.starred
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(id)
    }

    fn star_field(&self, object: &mut Map<String, Value>, id: &str) {
        if self.is_starred(id) {
            object.insert("starred".into(), STARRED_AT.into());
        }
    }

    fn song(&self, id: &str) -> Option<Value> {
        let (album, disc, track, title, artist) = find_song(id)?;
        let mut song = json!({
            "id": id,
            "title": title,
            "artist": artist,
            "album": album.title,
            "albumId": album.id,
            "track": track,
            "discNumber": disc,
            "duration": duration(id),
            "coverArt": album.id,
        });
        if let Value::Object(object) = &mut song {
            // 客座的署名不是曲库里的某一位歌手，上游不给 `artistId`。
            if artist == artist_name(album.artist) {
                object.insert("artistId".into(), album.artist.into());
            }
            self.star_field(object, id);
        }
        Some(song)
    }

    fn album(&self, album: &AlbumDef, with_songs: bool) -> Value {
        let tracks = songs(album);
        let mut value = json!({
            "id": album.id,
            "name": album.title,
            "artist": artist_name(album.artist),
            "artistId": album.artist,
            "coverArt": album.id,
            "songCount": tracks.len(),
            "duration": tracks.iter().map(|song| duration(&song.0)).sum::<u64>(),
            "year": album.year,
            "genre": album.genre,
        });
        let Value::Object(object) = &mut value else {
            return value;
        };
        self.star_field(object, album.id);
        if with_songs {
            let entries: Vec<_> = tracks
                .iter()
                .filter_map(|song| self.song(&song.0))
                .collect();
            object.insert("song".into(), entries.into());
            let titles: Vec<_> = album
                .disc_titles
                .iter()
                .enumerate()
                .map(|(index, title)| json!({"disc": index + 1, "title": title}))
                .collect();
            object.insert("discTitles".into(), titles.into());
        }
        value
    }

    fn artist(&self, id: &'static str, name: &'static str, with_albums: bool) -> Value {
        let albums: Vec<_> = ALBUMS.iter().filter(|album| album.artist == id).collect();
        let mut value = json!({
            "id": id,
            "name": name,
            "coverArt": id,
            "albumCount": albums.len(),
        });
        let Value::Object(object) = &mut value else {
            return value;
        };
        self.star_field(object, id);
        if with_albums {
            let albums: Vec<_> = albums
                .iter()
                .map(|album| self.album(album, false))
                .collect();
            object.insert("album".into(), albums.into());
        }
        value
    }

    fn playlist(&self, playlist: &PlaylistDef, with_songs: bool) -> Value {
        let mut value = json!({
            "id": playlist.id,
            "name": playlist.name,
            "owner": playlist.owner,
            "readonly": playlist.readonly,
            "songCount": playlist.songs.len(),
            "duration": playlist.songs.iter().map(|id| duration(id)).sum::<u64>(),
            "coverArt": playlist.id,
        });
        let Value::Object(object) = &mut value else {
            return value;
        };
        if let Some(comment) = playlist.comment {
            object.insert("comment".into(), comment.into());
        }
        if with_songs {
            let entries: Vec<_> = playlist
                .songs
                .iter()
                .filter_map(|id| self.song(id))
                .collect();
            object.insert("entry".into(), entries.into());
        }
        value
    }

    pub(super) fn respond(&self, method: &str, params: &Params) -> Reply {
        let id = params.get("id").map(String::as_str).unwrap_or_default();
        match method {
            "getAlbumList2" => self.album_list(params),
            "getPlaylists" => {
                let lists: Vec<_> = PLAYLISTS
                    .iter()
                    .map(|list| self.playlist(list, false))
                    .collect();
                one("playlists", json!({ "playlist": lists }))
            }
            "getArtists" => self.artists(),
            "search3" => self.search(params),
            "getStarred2" => self.starred(),
            "getGenres" => genres(),
            "getAlbum" => match ALBUMS.iter().find(|album| album.id == id) {
                Some(album) => one("album", self.album(album, true)),
                None => Reply::Failed(NOT_FOUND, "Album not found"),
            },
            "getArtist" => match ARTISTS.iter().find(|artist| artist.id == id) {
                Some(artist) => one("artist", self.artist(artist.id, artist.name, true)),
                None => Reply::Failed(NOT_FOUND, "Artist not found"),
            },
            "getPlaylist" => match PLAYLISTS.iter().find(|list| list.id == id) {
                Some(list) => one("playlist", self.playlist(list, true)),
                None => Reply::Failed(NOT_FOUND, "Playlist not found"),
            },
            "getLyricsBySongId" => lyrics(id),
            "getCoverArt" => Reply::Image(super::cover::bmp(id, number(params, "size", 320))),
            "star" | "unstar" => self.star(method == "star", params),
            _ => Reply::Failed(0, "not in the fake library"),
        }
    }

    fn album_list(&self, params: &Params) -> Reply {
        let kind = params.get("type").map(String::as_str).unwrap_or_default();
        let mut albums: Vec<&AlbumDef> = match kind {
            "recent" => RECENT
                .iter()
                .filter_map(|id| ALBUMS.iter().find(|a| a.id == *id))
                .collect(),
            "frequent" => FREQUENT
                .iter()
                .filter_map(|id| ALBUMS.iter().find(|a| a.id == *id))
                .collect(),
            "byGenre" => ALBUMS
                .iter()
                .filter(|album| {
                    params
                        .get("genre")
                        .is_some_and(|genre| genre == album.genre)
                })
                .collect(),
            _ => ALBUMS.iter().collect(),
        };
        match kind {
            "newest" => albums.sort_by(|a, b| b.year.cmp(&a.year).then(a.id.cmp(b.id))),
            "alphabeticalByName" => albums.sort_by_key(|album| album.title.to_lowercase()),
            "alphabeticalByArtist" => {
                albums.sort_by_key(|album| artist_name(album.artist).to_lowercase())
            }
            "random" => albums.reverse(),
            _ => {}
        }
        let page: Vec<_> = albums
            .into_iter()
            .skip(number(params, "offset", 0))
            .take(number(params, "size", 10))
            .map(|album| self.album(album, false))
            .collect();
        one("albumList2", json!({ "album": page }))
    }

    fn artists(&self) -> Reply {
        let mut index: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for artist in ARTISTS {
            let letter = artist
                .name
                .chars()
                .next()
                .filter(char::is_ascii_alphabetic)
                .map_or_else(|| "#".to_owned(), |c| c.to_ascii_uppercase().to_string());
            index
                .entry(letter)
                .or_default()
                .push(self.artist(artist.id, artist.name, false));
        }
        let index: Vec<_> = index
            .into_iter()
            .map(|(name, artists)| json!({"name": name, "artist": artists}))
            .collect();
        one("artists", json!({ "index": index }))
    }

    fn all_song_ids() -> impl Iterator<Item = String> {
        ALBUMS
            .iter()
            .flat_map(|album| songs(album).into_iter().map(|song| song.0))
    }

    fn search(&self, params: &Params) -> Reply {
        let query = params
            .get("query")
            .map(|q| q.to_lowercase())
            .unwrap_or_default();
        let hit = |text: &str| query.is_empty() || text.to_lowercase().contains(&query);
        let artists: Vec<_> = ARTISTS
            .iter()
            .filter(|artist| hit(artist.name))
            .take(number(params, "artistCount", 20))
            .map(|artist| self.artist(artist.id, artist.name, false))
            .collect();
        let albums: Vec<_> = ALBUMS
            .iter()
            .filter(|album| hit(album.title) || hit(artist_name(album.artist)))
            .take(number(params, "albumCount", 20))
            .map(|album| self.album(album, false))
            .collect();
        let songs: Vec<_> = Self::all_song_ids()
            .filter_map(|id| self.song(&id))
            .filter(|song| {
                hit(song["title"].as_str().unwrap_or_default())
                    || hit(song["artist"].as_str().unwrap_or_default())
            })
            .skip(number(params, "songOffset", 0))
            .take(number(params, "songCount", 20))
            .collect();
        one(
            "searchResult3",
            json!({"artist": artists, "album": albums, "song": songs}),
        )
    }

    fn starred(&self) -> Reply {
        let artists: Vec<_> = ARTISTS
            .iter()
            .filter(|artist| self.is_starred(artist.id))
            .map(|artist| self.artist(artist.id, artist.name, false))
            .collect();
        let albums: Vec<_> = ALBUMS
            .iter()
            .filter(|album| self.is_starred(album.id))
            .map(|album| self.album(album, false))
            .collect();
        let songs: Vec<_> = Self::all_song_ids()
            .filter(|id| self.is_starred(id))
            .filter_map(|id| self.song(&id))
            .collect();
        one(
            "starred2",
            json!({"artist": artists, "album": albums, "song": songs}),
        )
    }

    fn star(&self, starred: bool, params: &Params) -> Reply {
        let mut set = self
            .starred
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for key in ["id", "albumId", "artistId"] {
            if let Some(id) = params.get(key) {
                if starred {
                    set.insert(id.clone());
                } else {
                    set.remove(id);
                }
            }
        }
        Reply::Ok(Map::new())
    }
}

fn genres() -> Reply {
    let names: BTreeSet<_> = ALBUMS.iter().map(|album| album.genre).collect();
    let genres: Vec<_> = names
        .into_iter()
        .map(|genre| {
            let albums: Vec<_> = ALBUMS.iter().filter(|album| album.genre == genre).collect();
            json!({
                "value": genre,
                "albumCount": albums.len(),
                "songCount": albums.iter().map(|album| album.tracks.len()).sum::<usize>(),
            })
        })
        .collect();
    one("genres", json!({ "genre": genres }))
}

fn lyrics(id: &str) -> Reply {
    let (synced, lines): (bool, Vec<Value>) = if id == SYNCED_LYRICS.0 {
        let lines = SYNCED_LYRICS
            .1
            .iter()
            .map(|(start, text)| json!({"start": start, "value": text}))
            .collect();
        (true, lines)
    } else if id == PLAIN_LYRICS.0 {
        (
            false,
            PLAIN_LYRICS
                .1
                .iter()
                .map(|text| json!({ "value": text }))
                .collect(),
        )
    } else {
        return Reply::Failed(NOT_FOUND, "Lyrics not found");
    };
    one(
        "lyricsList",
        json!({"structuredLyrics": [{"synced": synced, "line": lines}]}),
    )
}
