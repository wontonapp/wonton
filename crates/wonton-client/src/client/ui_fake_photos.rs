//! `ui_fake` 的相册页夹具：一个三个月的假相册（最新一个月已打开、按日分好）、三种归属的相册、两个人物。
//! 页面上的操作照真核心的规则改视图（[`actions`]），不发请求；照片的字节现画一张图（[`picture`]），
//! 视频和实况的动态部分没有，如实报插件不可用。
//!
//! `WONTON_UI_FAKE_PHOTOS` 换开场：`source_required`（要先选相册插件）、`unavailable`（插件没在跑）、
//! `denied`（这个人没被授权）、`empty`（库是空的）、`share_denied`（有内容，但读不到家人名单，共享时没人可选）；
//! 不设就是有内容的库。

mod actions;
mod picture;
#[cfg(test)]
mod tests;

use std::sync::atomic::Ordering;

use wonton_protocol::media::library_view::{
    PhotoAlbumOwner, PhotoAlbumRow, PhotoAlbums, PhotoDay, PhotoEditKind, PhotoMonth, PhotoPeople,
    PhotoPerson, PhotoRendition, PhotoShareTarget, PhotoShareTargets, PhotoTile, PhotoTimeline,
    PhotosAction, PhotosViewModel,
};
use wonton_protocol::media::{MediaError, MediaFailure, MediaFailureCode, MediaKind};

use super::ClientCore;
use crate::photos::find_tile;

/// 2026-11-20T04:00Z。
const BASE_MS: u64 = 1_795_147_200_000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// 缩略图到之前的占位（ThumbHash）：只换了平均色的几张，格子不至于一个色；第一张是协议测试里的那张。
const THUMBHASHES: [&str; 7] = [
    "1QcSHQRnh493V4dIh4eXh1h4kJUI",
    "KIsRHQRnh493V4dIh4eXh1h4kJUI",
    "HkUSHQRnh493V4dIh4eXh1h4kJUI",
    "MOkRHQRnh493V4dIh4eXh1h4kJUI",
    "FocSHQRnh493V4dIh4eXh1h4kJUI",
    "JMoSHQRnh493V4dIh4eXh1h4kJUI",
    "LMYRHQRnh493V4dIh4eXh1h4kJUI",
];

/// 假相册里的人和地方；搜索的描述里带 [`NOTHING`] 就一条都搜不到。
const GRANDMA: &str = "奶奶";
const PLACE: [&str; 3] = ["杭州", "浙江", "中国"];
const NOTHING: &str = "没有";

/// 假相册的开场。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FakeMode {
    Library,
    SourceRequired,
    Unavailable,
    Denied,
    Empty,
    ShareDenied,
}

impl FakeMode {
    fn from_env() -> Self {
        match std::env::var("WONTON_UI_FAKE_PHOTOS").as_deref() {
            Ok("source_required") => Self::SourceRequired,
            Ok("unavailable") => Self::Unavailable,
            Ok("denied") => Self::Denied,
            Ok("empty") => Self::Empty,
            Ok("share_denied") => Self::ShareDenied,
            _ => Self::Library,
        }
    }
}

impl ClientCore {
    /// 假数据入口的相册操作：照真核心的规则改视图。
    pub(super) fn dispatch_ui_fake_photos(&self, action: PhotosAction) {
        let mode = FakeMode::from_env();
        self.views()
            .photos
            .send_modify(|view| actions::apply(view, action, mode));
    }

    /// 假数据入口要一条的字节：照片的三档现画一张图放进数据目录，回 `file://` 地址；视频和实况的动态部分
    /// 没有，如实报。真核心回 `None`，照常走回环。
    pub(super) fn ui_fake_photo_url(
        &self,
        id: &str,
        rendition: PhotoRendition,
    ) -> Option<Result<String, MediaError>> {
        if !self.ui_fake.load(Ordering::SeqCst) {
            return None;
        }
        let long_side = match rendition {
            PhotoRendition::Thumbnail => 240,
            PhotoRendition::Preview | PhotoRendition::Original => 1280,
            PhotoRendition::Playback | PhotoRendition::Live => return Some(Err(no_motion())),
        };
        let aspect = find_tile(&self.views().photos.borrow(), id)
            .and_then(|tile| tile.aspect)
            .unwrap_or(1.0);
        Some(self.fake_picture(id, long_side, aspect))
    }

    /// 假数据入口要人物头像，规则同 [`ui_fake_photo_url`](Self::ui_fake_photo_url)。
    pub(super) fn ui_fake_person_url(&self, id: &str) -> Option<Result<String, MediaError>> {
        if !self.ui_fake.load(Ordering::SeqCst) {
            return None;
        }
        Some(self.fake_picture(&format!("person-{id}"), 160, 1.0))
    }

    fn fake_picture(&self, name: &str, long_side: u32, aspect: f64) -> Result<String, MediaError> {
        let file: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = self.root.join("ui-fake-photos");
        let path = dir.join(format!("{file}-{long_side}.bmp"));
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            if path.exists() {
                return Ok(());
            }
            std::fs::write(&path, picture::bmp(name, long_side, aspect))
        });
        written.map_err(|error| local_io(&error.to_string()))?;
        url::Url::from_file_path(&path)
            .map(String::from)
            .map_err(|()| local_io("the fake picture path is not absolute"))
    }
}

fn no_motion() -> MediaError {
    MediaFailure {
        code: MediaFailureCode::PluginUnavailable,
        message: "the ui-fake library has no video or live-photo bytes".into(),
        retry_after_secs: None,
    }
    .into()
}

fn local_io(message: &str) -> MediaError {
    MediaFailure {
        code: MediaFailureCode::LocalIo,
        message: format!("cannot write a fake picture: {message}"),
        retry_after_secs: None,
    }
    .into()
}

fn tile(id: &str, taken_at: u64, kind: MediaKind) -> PhotoTile {
    let pick = id.bytes().map(usize::from).sum::<usize>() % THUMBHASHES.len();
    PhotoTile {
        id: id.to_owned(),
        duration_ms: (kind == MediaKind::Video).then_some(12_000),
        kind,
        taken_at,
        aspect: Some(4.0 / 3.0),
        live: false,
        favorite: false,
        owner: None,
        thumbhash: Some(THUMBHASHES[pick].to_owned()),
    }
}

fn album(id: &str, title: &str, count: u32, owner: PhotoAlbumOwner, shared: bool) -> PhotoAlbumRow {
    PhotoAlbumRow {
        id: id.to_owned(),
        title: title.to_owned(),
        count,
        owner,
        shared,
        cover: Some(format!("fake-{id}-cover")),
        members: if shared {
            vec!["mom".to_owned()]
        } else {
            Vec::new()
        },
        updated_at: Some(BASE_MS),
    }
}

/// 最新一个月：三天，带实况、视频、收藏和家人的各一条。
fn november() -> PhotoMonth {
    let photo = |id: &str, days_ago: u64| tile(id, BASE_MS - days_ago * DAY_MS, MediaKind::Photo);
    let mut live = photo("fake-live", 0);
    live.live = true;
    let mut favorite = photo("fake-favorite", 1);
    favorite.favorite = true;
    let mut from_mom = photo("fake-from-mom", 1);
    from_mom.owner = Some("mom".to_owned());
    from_mom.aspect = Some(3.0 / 4.0);
    let mut tall = photo("fake-tall", 2);
    tall.aspect = Some(9.0 / 16.0);
    let day = |date: &str, tiles: Vec<PhotoTile>| PhotoDay {
        date: date.to_owned(),
        tiles,
    };
    let days = vec![
        day(
            "2026-11-20",
            vec![
                live,
                tile("fake-video", BASE_MS, MediaKind::Video),
                photo("fake-a", 0),
                photo("fake-a2", 0),
                photo("fake-a3", 0),
            ],
        ),
        day(
            "2026-11-19",
            vec![favorite, from_mom, photo("fake-b", 1), photo("fake-b2", 1)],
        ),
        day(
            "2026-11-18",
            vec![
                tall,
                photo("fake-c", 2),
                photo("fake-c2", 2),
                photo("fake-c3", 2),
                photo("fake-c4", 2),
                photo("fake-c5", 2),
                photo("fake-c6", 2),
            ],
        ),
    ];
    PhotoMonth {
        month: "2026-11".to_owned(),
        count: 16,
        opened: true,
        days,
        ..PhotoMonth::default()
    }
}

/// 能共享给的家人：相册成员里的 `mom` 也在。
fn family() -> PhotoShareTargets {
    let person = |username: &str, display_name: &str| PhotoShareTarget {
        username: username.to_owned(),
        display_name: display_name.to_owned(),
    };
    PhotoShareTargets {
        loading: false,
        problem: None,
        people: vec![person("mom", "妈妈"), person("dad", "爸爸")],
    }
}

fn all_actions() -> Vec<PhotoEditKind> {
    vec![
        PhotoEditKind::Favorite,
        PhotoEditKind::Unfavorite,
        PhotoEditKind::CreateAlbum,
        PhotoEditKind::AddToAlbum,
        PhotoEditKind::RemoveFromAlbum,
        PhotoEditKind::ShareAlbum,
        PhotoEditKind::Trash,
        PhotoEditKind::Restore,
    ]
}

/// 有内容的假相册。
fn library() -> PhotosViewModel {
    let month = |month: &str, count| PhotoMonth {
        month: month.to_owned(),
        count,
        ..PhotoMonth::default()
    };
    PhotosViewModel {
        timeline: PhotoTimeline {
            loading: false,
            problem: None,
            months: vec![november(), month("2026-10", 120), month("2026-08", 37)],
        },
        albums: PhotoAlbums {
            loading: false,
            problem: None,
            mine: vec![
                album("trip", "海边", 14, PhotoAlbumOwner::Me, true),
                album("cat", "猫", 31, PhotoAlbumOwner::Me, false),
            ],
            household: vec![album("home", "家", 52, PhotoAlbumOwner::Household, true)],
            shared_with_me: vec![album(
                "mom-garden",
                "院子",
                8,
                PhotoAlbumOwner::Person {
                    username: "mom".to_owned(),
                },
                true,
            )],
        },
        people: PhotoPeople {
            loading: false,
            problem: None,
            people: vec![
                PhotoPerson {
                    id: "fake-person-1".to_owned(),
                    name: GRANDMA.to_owned(),
                },
                PhotoPerson {
                    id: "fake-person-2".to_owned(),
                    name: "小米".to_owned(),
                },
            ],
        },
        share_targets: family(),
        actions: Some(all_actions()),
        ..PhotosViewModel::default()
    }
}

/// 整页取不到的开场：三块都带同一个原因，写操作表没取到。
fn failed(code: MediaFailureCode, message: &str) -> PhotosViewModel {
    let problem = Some(MediaFailure {
        code,
        message: message.to_owned(),
        retry_after_secs: None,
    });
    PhotosViewModel {
        timeline: PhotoTimeline {
            problem: problem.clone(),
            ..PhotoTimeline::default()
        },
        albums: PhotoAlbums {
            problem: problem.clone(),
            ..PhotoAlbums::default()
        },
        people: PhotoPeople {
            problem: problem.clone(),
            ..PhotoPeople::default()
        },
        actions_problem: problem,
        ..PhotosViewModel::default()
    }
}

pub(super) fn opening(mode: FakeMode) -> PhotosViewModel {
    match mode {
        FakeMode::Library => library(),
        FakeMode::SourceRequired => failed(
            MediaFailureCode::SourceRequired,
            "more than one media source provides library; pass ?source=",
        ),
        FakeMode::Unavailable => failed(
            MediaFailureCode::PluginUnavailable,
            "503 from the photo plugin: upstream is not responding",
        ),
        FakeMode::Denied => failed(
            MediaFailureCode::PermissionDenied,
            "403: this person is not on the app's access list",
        ),
        FakeMode::Empty => PhotosViewModel {
            share_targets: family(),
            actions: Some(all_actions()),
            ..PhotosViewModel::default()
        },
        FakeMode::ShareDenied => PhotosViewModel {
            share_targets: PhotoShareTargets {
                problem: Some(MediaFailure {
                    code: MediaFailureCode::PermissionDenied,
                    message: "403: reading the member list needs manage_members".to_owned(),
                    retry_after_secs: None,
                }),
                ..PhotoShareTargets::default()
            },
            ..library()
        },
    }
}

pub(super) fn fake_photos() -> PhotosViewModel {
    opening(FakeMode::from_env())
}
