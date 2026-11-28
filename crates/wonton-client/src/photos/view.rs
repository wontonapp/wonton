//! 把提供方的回包并进相册页的视图：按日分组、去重、按归属分相册、写操作之后改格子。都是纯函数，不碰网络。

use std::collections::HashSet;

use wonton_protocol::media::library::{
    Albums, LibraryActionKind, LibraryActionResult, LibraryFeatures, PhotoAsset, Timeline,
};
use wonton_protocol::media::library_view::{
    PhotoAlbumOwner, PhotoAlbumRow, PhotoAlbums, PhotoDay, PhotoEdit, PhotoEditKind,
    PhotoEditOutcome, PhotoItemFailure, PhotoMonth, PhotoPage, PhotoTile, PhotosViewModel,
};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// 提供方接的写操作翻成壳出按钮用的种类：收藏一项对两个方向；这一版做不了的新动作不列。
pub(super) fn edit_kinds(features: &LibraryFeatures) -> Vec<PhotoEditKind> {
    let mut kinds = Vec::new();
    for action in &features.actions {
        let more: &[PhotoEditKind] = match action {
            LibraryActionKind::Favorite => &[PhotoEditKind::Favorite, PhotoEditKind::Unfavorite],
            LibraryActionKind::CreateAlbum => &[PhotoEditKind::CreateAlbum],
            LibraryActionKind::AddToAlbum => &[PhotoEditKind::AddToAlbum],
            LibraryActionKind::RemoveFromAlbum => &[PhotoEditKind::RemoveFromAlbum],
            LibraryActionKind::ShareAlbum => &[PhotoEditKind::ShareAlbum],
            LibraryActionKind::Trash => &[PhotoEditKind::Trash],
            LibraryActionKind::Restore => &[PhotoEditKind::Restore],
            LibraryActionKind::Other(_) => &[],
        };
        for kind in more {
            if !kinds.contains(kind) {
                kinds.push(*kind);
            }
        }
    }
    kinds
}

/// 按手机时差算的拍摄日 `YYYY-MM-DD`。
pub(super) fn day_of(taken_at: u64, utc_offset_minutes: i32) -> String {
    let local =
        i64::try_from(taken_at).unwrap_or(i64::MAX) + i64::from(utc_offset_minutes) * 60 * 1000;
    let (year, month, day) = civil_from_days(local.div_euclid(DAY_MS));
    format!("{year:04}-{month:02}-{day:02}")
}

/// 1970-01-01 起的天数换成公历年月日（Howard Hinnant 的 `civil_from_days`）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// 新的一页并进按日分组的格子：已在的 id 跳过（游标重叠时不出两格），同一天的接在那一天后面。
pub(super) fn append_days(days: &mut Vec<PhotoDay>, assets: &[PhotoAsset], offset: i32) {
    let mut seen: HashSet<String> = days
        .iter()
        .flat_map(|day| day.tiles.iter().map(|tile| tile.id.clone()))
        .collect();
    for asset in assets {
        if !seen.insert(asset.id.clone()) {
            continue;
        }
        let date = day_of(asset.taken_at, offset);
        let tile = PhotoTile::from(asset);
        match days.iter_mut().rev().find(|day| day.date == date) {
            Some(day) => day.tiles.push(tile),
            None => days.push(PhotoDay {
                date,
                tiles: vec![tile],
            }),
        }
    }
}

/// 相册、搜索的一页接到后面，同样按 id 去重。
pub(super) fn append_tiles(tiles: &mut Vec<PhotoTile>, assets: &[PhotoAsset]) {
    let mut seen: HashSet<String> = tiles.iter().map(|tile| tile.id.clone()).collect();
    tiles.extend(
        assets
            .iter()
            .filter(|asset| seen.insert(asset.id.clone()))
            .map(PhotoTile::from),
    );
}

/// 新的分桶换上去；打开过的月留着已取的格子，不在了的月去掉。
pub(super) fn merge_timeline(months: &mut Vec<PhotoMonth>, timeline: &Timeline) {
    let mut previous = std::mem::take(months);
    *months = timeline
        .buckets
        .iter()
        .map(|bucket| {
            match previous
                .iter()
                .position(|month| month.month == bucket.month)
            {
                Some(index) => {
                    let mut month = previous.swap_remove(index);
                    month.count = bucket.count;
                    month
                }
                None => PhotoMonth {
                    month: bucket.month.clone(),
                    count: bucket.count,
                    ..PhotoMonth::default()
                },
            }
        })
        .collect();
}

/// 相册按归属分三组，组内照提供方的顺序。
pub(super) fn sort_albums(view: &mut PhotoAlbums, albums: &Albums) {
    view.mine.clear();
    view.household.clear();
    view.shared_with_me.clear();
    for album in &albums.albums {
        insert_album(view, PhotoAlbumRow::from(album));
    }
}

/// 一行放进它归属的那组：同组里已有就原地换掉（不打乱顺序），归属变了就从旧组挪过来。
pub(super) fn insert_album(view: &mut PhotoAlbums, row: PhotoAlbumRow) {
    let group = match row.owner {
        PhotoAlbumOwner::Me => 0,
        PhotoAlbumOwner::Household => 1,
        PhotoAlbumOwner::Person { .. }
        | PhotoAlbumOwner::Unknown
        | PhotoAlbumOwner::Other { .. } => 2,
    };
    let mut groups = [
        &mut view.mine,
        &mut view.household,
        &mut view.shared_with_me,
    ];
    for (index, albums) in groups.iter_mut().enumerate() {
        if index != group {
            albums.retain(|album| album.id != row.id);
        }
    }
    let target = &mut groups[group];
    match target.iter_mut().find(|album| album.id == row.id) {
        Some(existing) => *existing = row,
        None => target.push(row),
    }
}

/// 列表里这个 id 的那一行。
pub(crate) fn find_album(view: &PhotoAlbums, id: &str) -> Option<PhotoAlbumRow> {
    view.mine
        .iter()
        .chain(&view.household)
        .chain(&view.shared_with_me)
        .find(|album| album.id == id)
        .cloned()
}

/// 网格里任一处的这一格（时间线、相册、搜索）。
pub(crate) fn find_tile(view: &PhotosViewModel, id: &str) -> Option<PhotoTile> {
    let months = view
        .timeline
        .months
        .iter()
        .flat_map(|month| &month.days)
        .flat_map(|day| &day.tiles);
    let pages = view
        .album
        .iter()
        .map(|album| &album.page)
        .chain(view.search.iter().map(|search| &search.page))
        .flat_map(|page| &page.tiles);
    months.chain(pages).find(|tile| tile.id == id).cloned()
}

/// 改每一处的这些格子。
fn for_tiles(
    view: &mut PhotosViewModel,
    ids: &HashSet<&str>,
    mut change: impl FnMut(&mut PhotoTile),
) {
    let mut touch = |tile: &mut PhotoTile| {
        if ids.contains(tile.id.as_str()) {
            change(tile);
        }
    };
    for month in &mut view.timeline.months {
        month
            .days
            .iter_mut()
            .flat_map(|day| &mut day.tiles)
            .for_each(&mut touch);
    }
    for page in pages(view) {
        page.tiles.iter_mut().for_each(&mut touch);
    }
    if let Some(tile) = view.detail.as_mut().and_then(|detail| detail.tile.as_mut()) {
        touch(tile);
    }
}

fn pages(view: &mut PhotosViewModel) -> impl Iterator<Item = &mut PhotoPage> {
    view.album
        .as_mut()
        .map(|album| &mut album.page)
        .into_iter()
        .chain(view.search.as_mut().map(|search| &mut search.page))
}

/// 进了回收站的格子从各处拿掉，月的张数跟着减；大图页正开着它就关掉。
fn remove_tiles(view: &mut PhotosViewModel, ids: &HashSet<&str>) {
    for month in &mut view.timeline.months {
        let before: usize = month.days.iter().map(|day| day.tiles.len()).sum();
        for day in &mut month.days {
            day.tiles.retain(|tile| !ids.contains(tile.id.as_str()));
        }
        month.days.retain(|day| !day.tiles.is_empty());
        let after: usize = month.days.iter().map(|day| day.tiles.len()).sum();
        let removed = u32::try_from(before - after).unwrap_or(u32::MAX);
        month.count = month.count.saturating_sub(removed);
    }
    for page in pages(view) {
        page.tiles.retain(|tile| !ids.contains(tile.id.as_str()));
    }
    if view
        .detail
        .as_ref()
        .is_some_and(|detail| ids.contains(detail.id.as_str()))
    {
        view.detail = None;
    }
}

/// 写操作回来了：结果记成轻提示，成了的那几条在格子上改掉，选择集只留没成的。
pub(crate) fn apply_edit(
    view: &mut PhotosViewModel,
    edit: &PhotoEdit,
    result: &LibraryActionResult,
) {
    let failed: HashSet<&str> = result.failed.iter().map(|item| item.id.as_str()).collect();
    let done: Vec<String> = edit
        .ids()
        .iter()
        .filter(|id| !failed.contains(id.as_str()))
        .cloned()
        .collect();
    let done_set: HashSet<&str> = done.iter().map(String::as_str).collect();
    match edit {
        PhotoEdit::Favorite { favorite, .. } => {
            for_tiles(view, &done_set, |tile| tile.favorite = *favorite);
        }
        PhotoEdit::Trash { .. } => remove_tiles(view, &done_set),
        PhotoEdit::RemoveFromAlbum { album, .. } => {
            if let Some(open) = view.album.as_mut().filter(|open| open.id == *album) {
                open.page
                    .tiles
                    .retain(|tile| !done_set.contains(tile.id.as_str()));
            }
        }
        PhotoEdit::CreateAlbum { .. }
        | PhotoEdit::AddToAlbum { .. }
        | PhotoEdit::ShareAlbum { .. }
        | PhotoEdit::Restore { .. } => {}
    }
    let album = result.album.as_ref().map(PhotoAlbumRow::from);
    if let Some(row) = &album {
        insert_album(&mut view.albums, row.clone());
        if let Some(open) = view.album.as_mut().filter(|open| open.id == row.id) {
            open.album = Some(row.clone());
        }
    }
    if !edit.ids().is_empty() {
        view.selected.retain(|id| failed.contains(id.as_str()));
    }
    view.last_edit = Some(PhotoEditOutcome {
        kind: edit.kind(),
        done,
        changed: result.changed,
        failed: result
            .failed
            .iter()
            .map(|item| PhotoItemFailure {
                id: item.id.clone(),
                failure: item.failure.clone(),
            })
            .collect(),
        album,
        problem: None,
    });
}

/// 选中或取消选中，保持选中的先后、不重复。
pub(crate) fn select(selected: &mut Vec<String>, ids: Vec<String>, on: bool) {
    if on {
        for id in ids {
            if !selected.contains(&id) {
                selected.push(id);
            }
        }
    } else {
        selected.retain(|id| !ids.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use wonton_protocol::media::MediaKind;

    use super::*;

    fn asset(id: &str, taken_at: u64) -> PhotoAsset {
        PhotoAsset {
            id: id.into(),
            kind: MediaKind::Photo,
            taken_at,
            aspect: None,
            duration_ms: None,
            live: false,
            favorite: false,
            owner: None,
            owner_unknown: false,
            thumbhash: None,
        }
    }

    #[test]
    fn days_follow_the_offset_across_midnight() {
        // 2026-11-26T16:30Z 在东八区已是 11-27。
        let at = 1_795_710_600_000;
        assert_eq!(day_of(at, 0), "2026-11-26");
        assert_eq!(day_of(at, 480), "2026-11-27");
        assert_eq!(day_of(0, -60), "1969-12-31");
        assert_eq!(day_of(951_782_400_000, 0), "2000-02-29");
    }

    #[test]
    fn pages_merge_into_days_without_duplicates() {
        let mut days = Vec::new();
        append_days(
            &mut days,
            &[asset("a", 1_795_710_600_000), asset("b", 1_795_700_000_000)],
            480,
        );
        append_days(
            &mut days,
            &[asset("b", 1_795_700_000_000), asset("c", 1_795_690_000_000)],
            480,
        );
        let dates: Vec<_> = days.iter().map(|day| day.date.as_str()).collect();
        assert_eq!(dates, ["2026-11-27", "2026-11-26"]);
        let ids: Vec<_> = days
            .iter()
            .flat_map(|day| day.tiles.iter().map(|tile| tile.id.as_str()))
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }
}
