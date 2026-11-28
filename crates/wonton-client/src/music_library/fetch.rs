//! 每一页从曲库取什么：一页对应一两个 OpenSubsonic 端点。

use serde::de::DeserializeOwned;
use wonton_protocol::music::{
    AlbumSort, MUSIC_HOME_ROW, MUSIC_PAGE_SIZE, MusicContent, MusicHome, MusicLyrics, MusicPage,
    MusicSearch, MusicStarred, MusicTarget,
};

use super::api::{CODE_WRONG_CREDENTIALS, Endpoint, Failure};
use super::{map, wire};

/// 搜索页每类最多几条：搜索页不翻页，太长的结果让人改词。
const SEARCH_ARTISTS: &str = "10";
const SEARCH_ALBUMS: &str = "20";
const SEARCH_SONGS: &str = "40";

/// 取到的一页（或下一页）。
pub(super) struct Loaded {
    pub(super) content: MusicContent,
    pub(super) has_more: bool,
}

/// 调一次端点；曲库还不认这个人时先建号再试一次。
async fn call<T: DeserializeOwned + Default>(
    endpoint: &Endpoint,
    method: &str,
    params: &[(&str, &str)],
    key: &str,
) -> Result<T, Failure> {
    match endpoint.call(method, params, key).await {
        Err(Failure::Subsonic { code, .. }) if code == CODE_WRONG_CREDENTIALS => {
            endpoint.sign_in().await?;
            endpoint.call(method, params, key).await
        }
        result => result,
    }
}

fn sort_type(sort: AlbumSort) -> &'static str {
    match sort {
        AlbumSort::Newest => "newest",
        AlbumSort::Recent => "recent",
        AlbumSort::Frequent => "frequent",
        AlbumSort::Name => "alphabeticalByName",
        AlbumSort::Artist => "alphabeticalByArtist",
        AlbumSort::Random => "random",
    }
}

async fn album_list(
    endpoint: &Endpoint,
    kind: &str,
    size: u32,
    offset: u32,
    genre: Option<&str>,
) -> Result<Vec<wire::Album>, Failure> {
    let (size, offset) = (size.to_string(), offset.to_string());
    let mut params = vec![("type", kind), ("size", &size), ("offset", &offset)];
    if let Some(genre) = genre {
        params.push(("genre", genre));
    }
    let list: wire::AlbumList = call(endpoint, "getAlbumList2", &params, "albumList2").await?;
    Ok(list.albums)
}

async fn playlists(endpoint: &Endpoint) -> Result<Vec<wire::Playlist>, Failure> {
    let list: wire::Playlists = call(endpoint, "getPlaylists", &[], "playlists").await?;
    Ok(list.playlists)
}

async fn home(endpoint: &Endpoint) -> Result<MusicHome, Failure> {
    let (added, played, most, lists) = tokio::join!(
        album_list(endpoint, "newest", MUSIC_HOME_ROW, 0, None),
        album_list(endpoint, "recent", MUSIC_HOME_ROW, 0, None),
        album_list(endpoint, "frequent", MUSIC_HOME_ROW, 0, None),
        playlists(endpoint),
    );
    Ok(MusicHome {
        recently_added: map::albums(&added?),
        recently_played: map::albums(&played?),
        most_played: map::albums(&most?),
        playlists: lists?.iter().map(map::playlist).collect(),
    })
}

/// 一页里从第 `offset` 条起的内容。不翻页的页 `offset` 只会是 0。
pub(super) async fn page(
    endpoint: &Endpoint,
    page: &MusicPage,
    offset: u32,
) -> Result<Loaded, Failure> {
    let whole = |content| Loaded {
        content,
        has_more: false,
    };
    let paged = |len: usize, content| Loaded {
        content,
        has_more: len >= MUSIC_PAGE_SIZE as usize,
    };
    Ok(match page {
        MusicPage::Home => whole(MusicContent::Home {
            home: home(endpoint).await?,
        }),
        MusicPage::Albums { sort } => {
            let albums =
                album_list(endpoint, sort_type(*sort), MUSIC_PAGE_SIZE, offset, None).await?;
            paged(
                albums.len(),
                MusicContent::Albums {
                    albums: map::albums(&albums),
                },
            )
        }
        MusicPage::Genre { name } => {
            let albums =
                album_list(endpoint, "byGenre", MUSIC_PAGE_SIZE, offset, Some(name)).await?;
            paged(
                albums.len(),
                MusicContent::Albums {
                    albums: map::albums(&albums),
                },
            )
        }
        MusicPage::Songs => {
            let songs = all_songs(endpoint, offset).await?;
            paged(
                songs.len(),
                MusicContent::Songs {
                    songs: map::tracks(&songs),
                },
            )
        }
        MusicPage::Artists => {
            let indexes: wire::ArtistIndexes = call(endpoint, "getArtists", &[], "artists").await?;
            let artists: Vec<_> = indexes
                .index
                .iter()
                .flat_map(|i| &i.artists)
                .map(map::artist)
                .collect();
            whole(MusicContent::Artists { artists })
        }
        MusicPage::Playlists => whole(MusicContent::Playlists {
            playlists: playlists(endpoint)
                .await?
                .iter()
                .map(map::playlist)
                .collect(),
        }),
        MusicPage::Starred => {
            let found: wire::Found = call(endpoint, "getStarred2", &[], "starred2").await?;
            whole(MusicContent::Starred {
                starred: starred(&found),
            })
        }
        MusicPage::Genres => {
            let list: wire::Genres = call(endpoint, "getGenres", &[], "genres").await?;
            let mut genres: Vec<_> = list.genres.iter().map(map::genre).collect();
            genres.sort_by_key(|genre| genre.name.to_lowercase());
            whole(MusicContent::Genres { genres })
        }
        MusicPage::Album { id } => {
            let album: wire::Album = call(endpoint, "getAlbum", &[("id", id)], "album").await?;
            whole(MusicContent::Album {
                album: map::album_detail(&album),
            })
        }
        MusicPage::Artist { id } => {
            let artist: wire::Artist = call(endpoint, "getArtist", &[("id", id)], "artist").await?;
            whole(MusicContent::Artist {
                artist: map::artist_detail(&artist),
            })
        }
        MusicPage::Playlist { id } => {
            let list: wire::Playlist =
                call(endpoint, "getPlaylist", &[("id", id)], "playlist").await?;
            whole(MusicContent::Playlist {
                playlist: map::playlist_detail(&list),
            })
        }
        MusicPage::Search => whole(search(endpoint, "").await?),
    })
}

/// 全部歌曲：`search3` 的空词就是全部（Navidrome 0.64.2 实测），只要歌。
async fn all_songs(endpoint: &Endpoint, offset: u32) -> Result<Vec<wire::Song>, Failure> {
    let (count, offset) = (MUSIC_PAGE_SIZE.to_string(), offset.to_string());
    let params = [
        ("query", ""),
        ("artistCount", "0"),
        ("albumCount", "0"),
        ("songCount", count.as_str()),
        ("songOffset", offset.as_str()),
    ];
    let found: wire::Found = call(endpoint, "search3", &params, "searchResult3").await?;
    Ok(found.songs)
}

fn starred(found: &wire::Found) -> MusicStarred {
    MusicStarred {
        artists: map::artists(&found.artists),
        albums: map::albums(&found.albums),
        songs: map::tracks(&found.songs),
    }
}

/// 搜一个词；空词不发请求，回空结果。
pub(super) async fn search(endpoint: &Endpoint, query: &str) -> Result<MusicContent, Failure> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(MusicContent::Search {
            query: String::new(),
            results: MusicSearch {
                artists: Vec::new(),
                albums: Vec::new(),
                songs: Vec::new(),
            },
        });
    }
    let params = [
        ("query", query),
        ("artistCount", SEARCH_ARTISTS),
        ("albumCount", SEARCH_ALBUMS),
        ("songCount", SEARCH_SONGS),
    ];
    let found: wire::Found = call(endpoint, "search3", &params, "searchResult3").await?;
    let starred = starred(&found);
    Ok(MusicContent::Search {
        query: query.to_owned(),
        results: MusicSearch {
            artists: starred.artists,
            albums: starred.albums,
            songs: starred.songs,
        },
    })
}

pub(super) async fn set_star(
    endpoint: &Endpoint,
    target: &MusicTarget,
    starred: bool,
) -> Result<(), Failure> {
    let param = match target {
        MusicTarget::Song(id) => ("id", id.as_str()),
        MusicTarget::Album(id) => ("albumId", id.as_str()),
        MusicTarget::Artist(id) => ("artistId", id.as_str()),
    };
    let method = if starred { "star" } else { "unstar" };
    call::<()>(endpoint, method, &[param], "").await
}

/// 一首歌的歌词；曲库没有这首的歌词（错误码 70）时为空。
pub(super) async fn lyrics(
    endpoint: &Endpoint,
    song: &str,
) -> Result<Option<MusicLyrics>, Failure> {
    match call::<wire::LyricsList>(endpoint, "getLyricsBySongId", &[("id", song)], "lyricsList")
        .await
    {
        Ok(list) => Ok(map::lyrics(&list)),
        Err(Failure::Subsonic { code: 70, .. }) => Ok(None),
        Err(failure) => Err(failure),
    }
}

/// 封面字节；`size` 是边长像素。
pub(super) async fn cover(endpoint: &Endpoint, id: &str, size: u32) -> Result<Vec<u8>, Failure> {
    let size = size.to_string();
    let params = [("id", id), ("size", size.as_str())];
    match endpoint.bytes("getCoverArt", &params).await {
        Err(Failure::Subsonic { code, .. }) if code == CODE_WRONG_CREDENTIALS => {
            endpoint.sign_in().await?;
            endpoint.bytes("getCoverArt", &params).await
        }
        result => result,
    }
}
