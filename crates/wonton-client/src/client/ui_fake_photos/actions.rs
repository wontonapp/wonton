//! 假相册上的操作：开月、开相册、搜索、大图、选择和写操作，照真核心的规则改视图。别人的条目改不了，
//! 写操作碰到它就逐条回 `permission_denied`，其余照做：真提供方的部分失败就是这个形状。

use wonton_protocol::media::library::{ActionFailure, Album, AlbumOwner, LibraryActionResult};
use wonton_protocol::media::library_view::{
    PhotoAlbumOwner, PhotoAlbumRow, PhotoAlbumView, PhotoDay, PhotoDetailView, PhotoEdit,
    PhotoInfo, PhotoLocation, PhotoPage, PhotoPerson, PhotoSearch, PhotoSearchView,
    PhotoSourceState, PhotoTile, PhotosAction, PhotosViewModel,
};
use wonton_protocol::media::{MediaFailure, MediaFailureCode, MediaKind};

use super::{BASE_MS, DAY_MS, FakeMode, GRANDMA, NOTHING, PLACE, tile};
use crate::photos::{apply_edit, find_album, find_tile, name_people, select};

/// 一个月先给的条数，剩下的翻页再给。
const FIRST_PAGE: u32 = 48;

pub(super) fn apply(view: &mut PhotosViewModel, action: PhotosAction, mode: FakeMode) {
    change(view, action, mode);
    name_people(view);
}

fn change(view: &mut PhotosViewModel, action: PhotosAction, mode: FakeMode) {
    match action {
        PhotosAction::Open { source, .. } => {
            // “要先选相册插件”的开场：带上插件再来才有库。
            if mode == FakeMode::SourceRequired
                && source.is_some()
                && view.timeline.problem.is_some()
            {
                *view = super::opening(FakeMode::Library);
            }
        }
        // 两家里挑了一家：照真核心，记下来再取库。
        PhotosAction::ChooseSource { id } => {
            if view.sources.state == PhotoSourceState::Choose {
                *view = super::opening(FakeMode::Library);
            }
            view.sources = super::ready(Some(id));
        }
        PhotosAction::Refresh | PhotosAction::LoadMoreAlbum | PhotosAction::LoadMoreSearch => {}
        PhotosAction::OpenMonth { month } => open_month(view, &month, false),
        PhotosAction::LoadMoreMonth { month } => open_month(view, &month, true),
        PhotosAction::OpenAlbum { id } => {
            let album = find_album(&view.albums, &id);
            let count = album.as_ref().map_or(0, |album| album.count);
            let tiles = generated(&format!("fake-album-{id}"), 0, count, BASE_MS);
            view.album = Some(PhotoAlbumView {
                id,
                album,
                page: PhotoPage {
                    tiles,
                    ..PhotoPage::default()
                },
            });
        }
        PhotosAction::CloseAlbum => view.album = None,
        PhotosAction::Search { query } => {
            let tiles = search(view, &query);
            view.search = Some(PhotoSearchView {
                query,
                page: PhotoPage {
                    tiles,
                    ..PhotoPage::default()
                },
            });
        }
        PhotosAction::ClearSearch => view.search = None,
        PhotosAction::OpenDetail { id } => {
            let tile = find_tile(view, &id);
            view.detail = Some(PhotoDetailView {
                info: Some(info(&id, tile.as_ref())),
                id,
                tile,
                loading: false,
                problem: None,
            });
        }
        PhotosAction::CloseDetail => view.detail = None,
        PhotosAction::Select { ids, selected } => select(&mut view.selected, ids, selected),
        PhotosAction::ClearSelection => view.selected.clear(),
        PhotosAction::Edit { edit } => edit_library(view, &edit),
        PhotosAction::DismissEditOutcome => view.last_edit = None,
    }
}

/// `prefix-<序号>` 的一串格子，序号从 `from` 到 `to`（不含），一格比一格早一小时；每七格一段视频、每五格一张实况。
fn generated(prefix: &str, from: u32, to: u32, newest: u64) -> Vec<PhotoTile> {
    (from..to)
        .map(|index| {
            let kind = if index % 7 == 3 {
                MediaKind::Video
            } else {
                MediaKind::Photo
            };
            let taken_at = newest.saturating_sub(u64::from(index) * 60 * 60 * 1000);
            let mut tile = tile(&format!("{prefix}-{index}"), taken_at, kind);
            tile.live = index % 5 == 1 && tile.kind == MediaKind::Photo;
            tile.favorite = index % 9 == 4;
            if index % 4 == 2 {
                tile.aspect = Some(3.0 / 4.0);
            }
            tile
        })
        .collect()
}

/// `YYYY-MM` 那个月最后一天中午（UTC）的毫秒；格式不对回夹具的基准时刻。
fn month_end(month: &str) -> u64 {
    let parsed = month
        .split_once('-')
        .and_then(|(year, month)| Some((year.parse::<i64>().ok()?, month.parse::<i64>().ok()?)));
    let Some((year, month)) = parsed.filter(|(_, month)| (1..=12).contains(month)) else {
        return BASE_MS;
    };
    // 下个月 1 号往前一天。从 1970-01-01 起的天数用民用历的换算（公历，闰年在内）。
    let (year, month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted.rem_euclid(400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468 - 1;
    u64::try_from(days).map_or(BASE_MS, |days| days * DAY_MS + DAY_MS / 2)
}

/// 取这个月的第一页（`more` 时取剩下的），按日分组：一天八条。
fn open_month(view: &mut PhotosViewModel, name: &str, more: bool) {
    let Some(month) = view
        .timeline
        .months
        .iter_mut()
        .find(|month| month.month == name)
    else {
        return;
    };
    let loaded: usize = month.days.iter().map(|day| day.tiles.len()).sum();
    let loaded = u32::try_from(loaded).unwrap_or(u32::MAX);
    if (month.opened && !more) || (more && !month.has_more) {
        return;
    }
    let to = if more {
        month.count
    } else {
        month.count.min(FIRST_PAGE)
    };
    let newest = month_end(name);
    for (index, tile) in (loaded..to).zip(generated(&format!("fake-{name}"), loaded, to, newest)) {
        let day = u64::from(index / 8);
        let date = format!("{name}-{:02}", 28u64.saturating_sub(day).max(1));
        let tile = PhotoTile {
            taken_at: newest.saturating_sub(day * DAY_MS + u64::from(index % 8) * 60_000),
            ..tile
        };
        match month.days.last_mut().filter(|last| last.date == date) {
            Some(last) => last.tiles.push(tile),
            None => month.days.push(PhotoDay {
                date,
                tiles: vec![tile],
            }),
        }
    }
    month.opened = true;
    month.has_more = to < month.count;
}

/// 在取过的时间线里按种类和收藏筛；描述里写 [`NOTHING`] 或 `nothing` 时一条都搜不到，按人筛时只留前三条。
fn search(view: &PhotosViewModel, query: &PhotoSearch) -> Vec<PhotoTile> {
    let text = query.text.as_deref().unwrap_or_default();
    if text.contains(NOTHING) || text.contains("nothing") {
        return Vec::new();
    }
    let limit = if query.people.is_empty() {
        usize::MAX
    } else {
        3
    };
    view.timeline
        .months
        .iter()
        .flat_map(|month| &month.days)
        .flat_map(|day| &day.tiles)
        .filter(|tile| query.kind.as_ref().is_none_or(|kind| tile.kind == *kind))
        .filter(|tile| {
            query
                .favorite
                .is_none_or(|favorite| tile.favorite == favorite)
        })
        .take(limit)
        .cloned()
        .collect()
}

fn info(id: &str, tile: Option<&PhotoTile>) -> PhotoInfo {
    let portrait = tile
        .and_then(|tile| tile.aspect)
        .is_some_and(|aspect| aspect < 1.0);
    let (width, height) = if portrait { (3024, 4032) } else { (4032, 3024) };
    let video = tile.is_some_and(|tile| tile.kind == MediaKind::Video);
    PhotoInfo {
        width: Some(width),
        height: Some(height),
        file_name: Some(format!("{id}.{}", if video { "mov" } else { "heic" })),
        size: Some(if video { 48_600_000 } else { 3_480_000 }),
        description: None,
        camera: Some("iPhone 17 Pro".to_owned()),
        lens: Some("24 mm f/1.78".to_owned()),
        place: PLACE.map(str::to_owned).to_vec(),
        location: Some(PhotoLocation {
            lat: 30.2741,
            lon: 120.1551,
        }),
        people: vec![PhotoPerson {
            id: "fake-person-1".to_owned(),
            name: GRANDMA.to_owned(),
        }],
    }
}

fn wire_album(row: PhotoAlbumRow) -> Album {
    Album {
        id: row.id,
        title: row.title,
        count: row.count,
        owner: match row.owner {
            PhotoAlbumOwner::Me => AlbumOwner::Me,
            PhotoAlbumOwner::Household => AlbumOwner::Household,
            PhotoAlbumOwner::Person { user } => AlbumOwner::Person(user.user_id),
            PhotoAlbumOwner::Unknown => AlbumOwner::Unknown,
            PhotoAlbumOwner::Other { raw } => AlbumOwner::Other(raw),
        },
        shared: row.shared,
        cover: row.cover,
        members: row.members.into_iter().map(|user| user.user_id).collect(),
        updated_at: row.updated_at,
    }
}

fn edit_library(view: &mut PhotosViewModel, edit: &PhotoEdit) {
    let failed: Vec<ActionFailure> = edit
        .ids()
        .iter()
        .filter(|id| find_tile(view, id).is_some_and(|tile| tile.owner.is_some()))
        .map(|id| ActionFailure {
            id: id.clone(),
            failure: MediaFailure {
                code: MediaFailureCode::PermissionDenied,
                message: "403: the item belongs to another person".to_owned(),
                retry_after_secs: None,
            },
        })
        .collect();
    let changed = u32::try_from(edit.ids().len() - failed.len()).unwrap_or(u32::MAX);
    let existing = |id: &str| find_album(&view.albums, id).map(wire_album);
    let album = match edit {
        PhotoEdit::CreateAlbum {
            title,
            ids,
            share_with,
        } => Some(Album {
            id: format!("fake-new-{}", view.albums.mine.len()),
            title: title.trim().to_owned(),
            count: changed,
            owner: AlbumOwner::Me,
            shared: !share_with.is_empty(),
            cover: ids.first().cloned(),
            members: share_with.clone(),
            updated_at: Some(BASE_MS),
        }),
        PhotoEdit::AddToAlbum { album, .. } => existing(album).map(|mut album| {
            album.count += changed;
            album
        }),
        PhotoEdit::RemoveFromAlbum { album, .. } => existing(album).map(|mut album| {
            album.count = album.count.saturating_sub(changed);
            album
        }),
        PhotoEdit::ShareAlbum { album, people } => existing(album).map(|mut album| {
            album.shared = true;
            album.members.clone_from(people);
            album
        }),
        PhotoEdit::Favorite { .. } | PhotoEdit::Trash { .. } | PhotoEdit::Restore { .. } => None,
    };
    apply_edit(
        view,
        edit,
        &LibraryActionResult {
            changed,
            album,
            failed,
        },
    );
}
