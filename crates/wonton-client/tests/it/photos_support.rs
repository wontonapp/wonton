//! 假核心里的相册：`/_wonton/api/media/library/…` 按提供方的形状回时间线、相册、搜索、人物、详情和字节，
//! 写操作照真的记账（收藏、进回收站、恢复，建相册、加进、移出、共享），逐条失败按测试塞进来的回。
//!
//! 隧道和请求记录用 `plugins::rig` 的；要整个接口失败时用 rig 的预设回应，它排在这里前面。

use std::collections::HashMap;
use std::sync::Mutex;

use axum::body::Body;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use wonton_protocol::media::library::{
    ActionFailure, Album, AlbumOwner, AlbumPage, Albums, AssetPage, LibraryAction,
    LibraryActionKind, LibraryActionResult, LibraryFeatures, People, PersonEntry, PhotoAsset,
    PhotoDetail, Timeline, TimelineBucket,
};
use wonton_protocol::media::{MediaFailure, MediaFailureCode, MediaKind};

const PREFIX: &str = "/_wonton/api/media/library";

/// 每页几条：小，好测翻页。
const PAGE: usize = 2;

#[derive(Default)]
pub struct PhotosFake {
    /// 月 → 条目（新的在前）；进回收站的从这里挪到 `trash`。
    months: Mutex<Vec<(String, Vec<PhotoAsset>)>>,
    trash: Mutex<Vec<(String, PhotoAsset)>>,
    albums: Mutex<Vec<(Album, Vec<String>)>>,
    people: Mutex<Vec<PersonEntry>>,
    /// 搜索不管条件，回这一串。
    found: Mutex<Vec<PhotoAsset>>,
    /// (id, 档位) → 字节；人物头像的档位记成 `person`。
    bytes: Mutex<HashMap<(String, String), Vec<u8>>>,
    /// 写操作里这些 id 逐条失败。
    refuse: Mutex<HashMap<String, MediaFailure>>,
}

pub fn asset(id: &str, taken_at: u64) -> PhotoAsset {
    PhotoAsset {
        id: id.to_owned(),
        kind: MediaKind::Photo,
        taken_at,
        aspect: Some(1.5),
        duration_ms: None,
        live: false,
        favorite: false,
        owner: None,
        owner_unknown: false,
        thumbhash: None,
    }
}

pub fn album(id: &str, title: &str, owner: AlbumOwner) -> Album {
    Album {
        id: id.to_owned(),
        title: title.to_owned(),
        count: 0,
        owner,
        shared: false,
        cover: None,
        members: Vec::new(),
        updated_at: None,
    }
}

pub fn envelope(code: MediaFailureCode, message: &str) -> String {
    serde_json::to_string(&MediaFailure {
        code,
        message: message.to_owned(),
        retry_after_secs: None,
    })
    .unwrap()
}

impl PhotosFake {
    pub fn month(&self, month: &str, assets: Vec<PhotoAsset>) {
        self.months.lock().unwrap().push((month.to_owned(), assets));
    }

    pub fn album(&self, album: Album, ids: &[&str]) {
        let ids = ids.iter().map(|id| (*id).to_owned()).collect();
        self.albums.lock().unwrap().push((album, ids));
    }

    pub fn person(&self, id: &str, name: &str) {
        self.people.lock().unwrap().push(PersonEntry {
            id: id.to_owned(),
            name: name.to_owned(),
        });
    }

    pub fn found(&self, assets: Vec<PhotoAsset>) {
        *self.found.lock().unwrap() = assets;
    }

    pub fn bytes(&self, id: &str, rendition: &str, bytes: Vec<u8>) {
        self.bytes
            .lock()
            .unwrap()
            .insert((id.to_owned(), rendition.to_owned()), bytes);
    }

    pub fn refuse(&self, id: &str, code: MediaFailureCode) {
        self.refuse.lock().unwrap().insert(
            id.to_owned(),
            MediaFailure {
                code,
                message: format!("{id} refused"),
                retry_after_secs: None,
            },
        );
    }

    fn all(&self) -> Vec<PhotoAsset> {
        self.months
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(_, assets)| assets.clone())
            .collect()
    }

    fn find(&self, id: &str) -> Option<PhotoAsset> {
        self.all().into_iter().find(|asset| asset.id == id)
    }
}

fn json(value: impl serde::Serialize) -> Response {
    (StatusCode::OK, serde_json::to_string(&value).unwrap()).into_response()
}

fn query(uri: &Uri, key: &str) -> Option<String> {
    uri.query()?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| percent_decode(value))
    })
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(if bytes[index] == b'+' {
                    b' '
                } else {
                    bytes[index]
                });
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 游标是下一页的起点下标。
fn page(assets: &[PhotoAsset], cursor: Option<String>) -> AssetPage {
    let start: usize = cursor.and_then(|cursor| cursor.parse().ok()).unwrap_or(0);
    let end = (start + PAGE).min(assets.len());
    AssetPage {
        assets: assets.get(start..end).unwrap_or_default().to_vec(),
        cursor: (end < assets.len()).then(|| end.to_string()),
    }
}

/// 这个请求归相册接口时回一份回应；不归时 `None`。
pub fn respond(
    fake: &PhotosFake,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Option<Response> {
    let rest = uri.path().strip_prefix(PREFIX)?;
    let segments: Vec<String> = rest
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(percent_decode)
        .collect();
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
    let cursor = query(uri, "cursor");
    Some(match (method, segments.as_slice()) {
        // 假相册接的写操作，外加一个这一版不认识的。
        (&Method::GET, []) => json(LibraryFeatures {
            actions: vec![
                LibraryActionKind::Favorite,
                LibraryActionKind::CreateAlbum,
                LibraryActionKind::AddToAlbum,
                LibraryActionKind::RemoveFromAlbum,
                LibraryActionKind::ShareAlbum,
                LibraryActionKind::Trash,
                LibraryActionKind::Restore,
                LibraryActionKind::Other("rotate".to_owned()),
            ],
        }),
        (&Method::GET, ["timeline"]) => json(Timeline {
            buckets: fake
                .months
                .lock()
                .unwrap()
                .iter()
                .map(|(month, assets)| TimelineBucket {
                    month: month.clone(),
                    count: assets.len() as u32,
                })
                .collect(),
        }),
        (&Method::GET, ["timeline", month]) => {
            let months = fake.months.lock().unwrap();
            let (_, assets) = months.iter().find(|(name, _)| name == month)?;
            json(page(assets, cursor))
        }
        (&Method::GET, ["albums"]) => json(Albums {
            albums: fake
                .albums
                .lock()
                .unwrap()
                .iter()
                .map(|(album, ids)| Album {
                    count: ids.len() as u32,
                    ..album.clone()
                })
                .collect(),
        }),
        (&Method::GET, ["albums", id]) => album_page(fake, id, cursor)?,
        (&Method::GET, ["search"]) => json(page(&fake.found.lock().unwrap(), cursor)),
        (&Method::GET, ["people"]) => json(People {
            people: fake.people.lock().unwrap().clone(),
        }),
        (&Method::GET, ["people", id, "thumbnail"]) => bytes(fake, id, "person", headers)?,
        (&Method::GET, ["items", id]) => json(PhotoDetail {
            asset: fake.find(id)?,
            width: Some(4032),
            height: Some(3024),
            file_name: Some(format!("{id}.HEIC")),
            size: None,
            description: None,
            camera: Some("Phone".to_owned()),
            lens: None,
            place: vec!["青岛".to_owned()],
            location: None,
            people: Vec::new(),
        }),
        (&Method::GET, ["items", id, "content"]) => bytes(
            fake,
            id,
            &query(uri, "rendition").unwrap_or_default(),
            headers,
        )?,
        (&Method::POST, ["actions"]) => act(fake, serde_json::from_slice(body).ok()?),
        _ => return None,
    })
}

fn album_page(fake: &PhotosFake, id: &str, cursor: Option<String>) -> Option<Response> {
    let albums = fake.albums.lock().unwrap();
    let (album, ids) = albums.iter().find(|(album, _)| album.id == id)?;
    let assets: Vec<PhotoAsset> = ids.iter().filter_map(|id| fake.find(id)).collect();
    Some(json(AlbumPage {
        album: Album {
            count: ids.len() as u32,
            ..album.clone()
        },
        page: page(&assets, cursor),
    }))
}

/// 字节，认 `Range: bytes=a-b`。
fn bytes(fake: &PhotosFake, id: &str, rendition: &str, headers: &HeaderMap) -> Option<Response> {
    let bytes = fake
        .bytes
        .lock()
        .unwrap()
        .get(&(id.to_owned(), rendition.to_owned()))?
        .clone();
    let range = headers
        .get("range")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.split_once('-'))
        .and_then(|(from, to)| Some((from.parse::<usize>().ok()?, to.parse::<usize>().ok()?)));
    let builder = Response::builder().header("content-type", "image/jpeg");
    Some(match range {
        Some((from, to)) => builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                "content-range",
                format!("bytes {from}-{to}/{}", bytes.len()),
            )
            .body(Body::from(bytes[from..=to].to_vec()))
            .unwrap(),
        None => builder
            .status(StatusCode::OK)
            .body(Body::from(bytes))
            .unwrap(),
    })
}

/// 写操作：拒绝名单里的逐条失败，其余照做。
fn act(fake: &PhotosFake, action: LibraryAction) -> Response {
    let refuse = fake.refuse.lock().unwrap().clone();
    let mut failed = Vec::new();
    let mut allowed = |ids: Vec<String>| -> Vec<String> {
        ids.into_iter()
            .filter(|id| match refuse.get(id) {
                Some(failure) => {
                    failed.push(ActionFailure {
                        id: id.clone(),
                        failure: failure.clone(),
                    });
                    false
                }
                None => true,
            })
            .collect()
    };
    let mut album_result = None;
    let changed = match action {
        LibraryAction::Favorite { ids, favorite } => {
            let ids = allowed(ids);
            for (_, assets) in fake.months.lock().unwrap().iter_mut() {
                for asset in assets.iter_mut().filter(|asset| ids.contains(&asset.id)) {
                    asset.favorite = favorite;
                }
            }
            ids.len()
        }
        LibraryAction::Trash { ids } => {
            let ids = allowed(ids);
            let mut months = fake.months.lock().unwrap();
            for (month, assets) in months.iter_mut() {
                let (gone, kept): (Vec<_>, Vec<_>) =
                    assets.drain(..).partition(|asset| ids.contains(&asset.id));
                *assets = kept;
                fake.trash
                    .lock()
                    .unwrap()
                    .extend(gone.into_iter().map(|asset| (month.clone(), asset)));
            }
            ids.len()
        }
        LibraryAction::Restore { ids } => {
            let ids = allowed(ids);
            let mut trash = fake.trash.lock().unwrap();
            let (back, kept): (Vec<_>, Vec<_>) = trash
                .drain(..)
                .partition(|(_, asset)| ids.contains(&asset.id));
            *trash = kept;
            let mut months = fake.months.lock().unwrap();
            for (month, asset) in back {
                if let Some((_, assets)) = months.iter_mut().find(|(name, _)| *name == month) {
                    assets.push(asset);
                    assets.sort_by_key(|asset| std::cmp::Reverse(asset.taken_at));
                }
            }
            ids.len()
        }
        other => match album_act(fake, other, &mut allowed) {
            Ok((changed, album)) => {
                album_result = album;
                changed
            }
            Err(failure) => return failure.into_response(),
        },
    };
    json(LibraryActionResult {
        changed: changed as u32,
        album: album_result,
        failed,
    })
}

/// 相册上的写操作：建、加进、移出、共享。回改了几项和改过的相册。
fn album_act(
    fake: &PhotosFake,
    action: LibraryAction,
    allowed: &mut dyn FnMut(Vec<String>) -> Vec<String>,
) -> Result<(usize, Option<Album>), (StatusCode, String)> {
    let mut albums = fake.albums.lock().unwrap();
    if let LibraryAction::CreateAlbum {
        title,
        ids,
        share_with,
    } = action
    {
        let ids = allowed(ids);
        let created = Album {
            id: format!("new-{}", albums.len()),
            title,
            count: ids.len() as u32,
            owner: AlbumOwner::Me,
            shared: !share_with.is_empty(),
            cover: ids.first().cloned(),
            members: share_with,
            updated_at: None,
        };
        albums.push((created.clone(), ids.clone()));
        return Ok((ids.len(), Some(created)));
    }
    let target = match &action {
        LibraryAction::AddToAlbum { album, .. }
        | LibraryAction::RemoveFromAlbum { album, .. }
        | LibraryAction::ShareAlbum { album, .. } => album.clone(),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                envelope(MediaFailureCode::Protocol, "not in the fake"),
            ));
        }
    };
    let Some((found, items)) = albums.iter_mut().find(|(found, _)| found.id == target) else {
        return Err((
            StatusCode::NOT_FOUND,
            envelope(MediaFailureCode::MediaGone, "no album"),
        ));
    };
    let changed = match action {
        LibraryAction::AddToAlbum { ids, .. } => {
            let ids = allowed(ids);
            items.extend(ids.iter().cloned());
            ids.len()
        }
        LibraryAction::RemoveFromAlbum { ids, .. } => {
            let ids = allowed(ids);
            items.retain(|id| !ids.contains(id));
            ids.len()
        }
        LibraryAction::ShareAlbum { people, .. } => {
            found.shared = true;
            let new: Vec<String> = people
                .into_iter()
                .filter(|person| !found.members.contains(person))
                .collect();
            found.members.extend(new.iter().cloned());
            new.len()
        }
        _ => 0,
    };
    Ok((
        changed,
        Some(Album {
            count: items.len() as u32,
            ..found.clone()
        }),
    ))
}
