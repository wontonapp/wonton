//! 每一块怎么取、回来怎么写：时间线、月、相册、搜索、人物、大图页和写操作。

use std::sync::Arc;

use wonton_protocol::CoreError;
use wonton_protocol::media::MediaFailure;
use wonton_protocol::media::library::{
    AlbumPage, Albums, AssetPage, LibraryActionResult, LibraryFeatures, PATH_LIBRARY,
    PATH_LIBRARY_ACTIONS, PATH_LIBRARY_ALBUMS, PATH_LIBRARY_PEOPLE, PATH_LIBRARY_TIMELINE, People,
    PhotoDetail, Timeline, album_path, library_item_path, timeline_bucket_path,
};
use wonton_protocol::media::library_view::{
    PhotoAlbumRow, PhotoAlbumView, PhotoDetailView, PhotoEdit, PhotoEditOutcome, PhotoInfo,
    PhotoPage, PhotoPerson, PhotoSearch, PhotoSearchView, PhotoTile, PhotosViewModel,
};

use super::{PhotosLink, Section, api, view};
use crate::media_link::MediaApi;

/// 取第一页还是下一页。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    First,
    /// 重取第一页，打开过的也取（刷新、恢复）。
    Reload,
    More,
}

/// 壳发来的操作本身不对（壳的毛病，不是用户能改的），原因写给开发者看。
fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_owned(),
    }
}

/// 带上翻页游标的提供方路径。
fn paged(path: String, cursor: Option<&str>) -> String {
    api::with_query(path, &[("cursor", cursor)])
}

/// `YYYY-MM`。
fn is_month(month: &str) -> bool {
    let bytes = month.as_bytes();
    bytes.len() == 7
        && bytes[4] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || byte.is_ascii_digit())
}

fn page_done(
    page: &mut PhotoPage,
    result: Result<AssetPage, MediaFailure>,
    more: bool,
) -> Option<Option<String>> {
    page.loading = false;
    page.loading_more = false;
    match result {
        Ok(fetched) => {
            if !more {
                page.tiles.clear();
            }
            view::append_tiles(&mut page.tiles, &fetched.assets);
            page.has_more = fetched.cursor.is_some();
            page.problem = None;
            Some(fetched.cursor)
        }
        Err(failure) => {
            page.problem = Some(failure);
            None
        }
    }
}

/// 翻页能不能开：有下一页、不在取。
fn can_load_more(page: &PhotoPage) -> bool {
    page.has_more && !page.loading && !page.loading_more
}

impl PhotosLink {
    /// 写操作表、时间线、相册、人物和能共享给的家人。
    pub(super) fn refresh_lists(self: &Arc<Self>) {
        self.features();
        self.timeline();
        self.albums();
        self.people();
        self.share_targets();
    }

    /// 都重取：列表，加上打开着的月、相册、搜索和大图页的第一页。
    pub(super) fn refresh_all(self: &Arc<Self>) {
        self.refresh_lists();
        self.reload_open(|_| true);
    }

    /// 重新连上：列表重取，打开着的那几块只补取上次没取成的。
    pub(super) fn recover(self: &Arc<Self>) {
        self.refresh_lists();
        self.reload_open(|problem| problem.is_some());
    }

    fn reload_open(self: &Arc<Self>, pick: impl Fn(&Option<MediaFailure>) -> bool) {
        let current = self.view.borrow().clone();
        for month in &current.timeline.months {
            if month.opened && pick(&month.problem) {
                let _ = self.month(month.month.clone(), Page::Reload);
            }
        }
        if let Some(album) = current
            .album
            .as_ref()
            .filter(|album| pick(&album.page.problem))
        {
            self.album(Some(album.id.clone()), Page::Reload);
        }
        if let Some(search) = current
            .search
            .as_ref()
            .filter(|search| pick(&search.page.problem))
        {
            let _ = self.search(Some(search.query.clone()), Page::Reload);
        }
        if let Some(detail) = current
            .detail
            .as_ref()
            .filter(|detail| pick(&detail.problem))
        {
            self.detail(detail.id.clone());
        }
    }

    fn features(self: &Arc<Self>) {
        let Some(ticket) = self.start(Section::Features, |_, _| true) else {
            return;
        };
        self.fetch(
            ticket,
            PATH_LIBRARY.to_owned(),
            |_, view, result: Result<LibraryFeatures, _>| {
                view.actions = result.ok().map(|features| view::edit_kinds(&features));
            },
        );
    }

    fn timeline(self: &Arc<Self>) {
        let Some(ticket) = self.start(Section::Timeline, |_, view| {
            view.timeline.loading = true;
            true
        }) else {
            return;
        };
        self.fetch(
            ticket,
            PATH_LIBRARY_TIMELINE.to_owned(),
            |state, view, result: Result<Timeline, _>| {
                view.timeline.loading = false;
                match result {
                    Ok(timeline) => {
                        view::merge_timeline(&mut view.timeline.months, &timeline);
                        view.timeline.problem = None;
                        state.month_cursors.retain(|month, _| {
                            timeline.buckets.iter().any(|bucket| bucket.month == *month)
                        });
                    }
                    Err(failure) => view.timeline.problem = Some(failure),
                }
            },
        );
    }

    pub(super) fn albums(self: &Arc<Self>) {
        let Some(ticket) = self.start(Section::Albums, |_, view| {
            view.albums.loading = true;
            true
        }) else {
            return;
        };
        self.fetch(
            ticket,
            PATH_LIBRARY_ALBUMS.to_owned(),
            |_, view, result: Result<Albums, _>| {
                view.albums.loading = false;
                match result {
                    Ok(albums) => {
                        view::sort_albums(&mut view.albums, &albums);
                        view.albums.problem = None;
                    }
                    Err(failure) => view.albums.problem = Some(failure),
                }
            },
        );
    }

    fn people(self: &Arc<Self>) {
        let Some(ticket) = self.start(Section::People, |_, view| {
            view.people.loading = true;
            true
        }) else {
            return;
        };
        self.fetch(
            ticket,
            PATH_LIBRARY_PEOPLE.to_owned(),
            |_, view, result: Result<People, _>| {
                view.people.loading = false;
                match result {
                    Ok(people) => {
                        view.people.people = people
                            .people
                            .into_iter()
                            .map(|person| PhotoPerson {
                                id: person.id,
                                name: person.name,
                            })
                            .collect();
                        view.people.problem = None;
                    }
                    Err(failure) => view.people.problem = Some(failure),
                }
            },
        );
    }

    pub(super) fn month(self: &Arc<Self>, month: String, page: Page) -> Result<(), CoreError> {
        if !is_month(&month) {
            return Err(invalid("month must look like YYYY-MM"));
        }
        let mut cursor = None;
        let section = Section::Month(month.clone());
        let Some(ticket) = self.start(section, |state, view| {
            let Some(entry) = view
                .timeline
                .months
                .iter_mut()
                .find(|entry| entry.month == month)
            else {
                return false;
            };
            match page {
                Page::First if entry.opened && entry.problem.is_none() => return false,
                Page::More => {
                    let next = state.month_cursors.get(&month).cloned().flatten();
                    if next.is_none() || entry.loading || entry.loading_more {
                        return false;
                    }
                    cursor = next;
                    entry.loading_more = true;
                }
                Page::First | Page::Reload => entry.loading = true,
            }
            entry.opened = true;
            true
        }) else {
            return Ok(());
        };
        let more = page == Page::More;
        let key = month.clone();
        self.fetch(
            ticket,
            paged(timeline_bucket_path(&month), cursor.as_deref()),
            move |state, view, result: Result<AssetPage, _>| {
                let offset = state.utc_offset_minutes;
                let Some(entry) = view
                    .timeline
                    .months
                    .iter_mut()
                    .find(|entry| entry.month == key)
                else {
                    return;
                };
                entry.loading = false;
                entry.loading_more = false;
                match result {
                    Ok(fetched) => {
                        if !more {
                            entry.days.clear();
                        }
                        view::append_days(&mut entry.days, &fetched.assets, offset);
                        entry.has_more = fetched.cursor.is_some();
                        entry.problem = None;
                        state.month_cursors.insert(key, fetched.cursor);
                    }
                    Err(failure) => entry.problem = Some(failure),
                }
            },
        );
        Ok(())
    }

    /// 打开（`id` 有值）或翻下一页（`id` 为空，接着打开着的那个）。
    pub(super) fn album(self: &Arc<Self>, id: Option<String>, page: Page) {
        let mut cursor = None;
        let mut target = String::new();
        let Some(ticket) = self.start(Section::Album, |state, view| {
            match (page, id) {
                (Page::More, _) => {
                    let Some(open) = view.album.as_mut().filter(|open| can_load_more(&open.page))
                    else {
                        return false;
                    };
                    let Some(next) = state.album_cursor.clone() else {
                        return false;
                    };
                    cursor = Some(next);
                    open.page.loading_more = true;
                    target.clone_from(&open.id);
                }
                (_, Some(id)) => {
                    state.album_cursor = None;
                    view.album = Some(PhotoAlbumView {
                        album: view::find_album(&view.albums, &id),
                        id: id.clone(),
                        page: PhotoPage {
                            loading: true,
                            ..PhotoPage::default()
                        },
                    });
                    target = id;
                }
                (_, None) => return false,
            }
            true
        }) else {
            return;
        };
        let more = page == Page::More;
        self.fetch(
            ticket,
            paged(album_path(&target), cursor.as_deref()),
            move |state, view, result: Result<AlbumPage, _>| {
                let Some(open) = view.album.as_mut() else {
                    return;
                };
                let album: Option<PhotoAlbumRow> =
                    result.as_ref().ok().map(|fetched| (&fetched.album).into());
                if let Some(next) =
                    page_done(&mut open.page, result.map(|fetched| fetched.page), more)
                {
                    state.album_cursor = next;
                }
                if let Some(row) = album {
                    open.album = Some(row);
                }
            },
        );
    }

    /// 新搜（`query` 有值）或翻下一页。
    pub(super) fn search(
        self: &Arc<Self>,
        query: Option<PhotoSearch>,
        page: Page,
    ) -> Result<(), CoreError> {
        if query.as_ref().is_some_and(PhotoSearch::is_empty) {
            return Err(invalid("a search needs at least one condition"));
        }
        let mut wire = None;
        let Some(ticket) = self.start(Section::Search, |state, view| {
            match (page, query) {
                (Page::More, _) => {
                    let Some(open) = view
                        .search
                        .as_mut()
                        .filter(|open| can_load_more(&open.page))
                    else {
                        return false;
                    };
                    let Some(next) = state.search_cursor.clone() else {
                        return false;
                    };
                    wire = Some(open.query.to_query(Some(&next)));
                    open.page.loading_more = true;
                }
                (_, Some(query)) => {
                    state.search_cursor = None;
                    wire = Some(query.to_query(None));
                    view.search = Some(PhotoSearchView {
                        query,
                        page: PhotoPage {
                            loading: true,
                            ..PhotoPage::default()
                        },
                    });
                }
                (_, None) => return false,
            }
            true
        }) else {
            return Ok(());
        };
        let more = page == Page::More;
        let path = wire.unwrap_or_default().path();
        self.fetch(
            ticket,
            path,
            move |state, view, result: Result<AssetPage, _>| {
                let Some(open) = view.search.as_mut() else {
                    return;
                };
                if let Some(next) = page_done(&mut open.page, result, more) {
                    state.search_cursor = next;
                }
            },
        );
        Ok(())
    }

    pub(super) fn detail(self: &Arc<Self>, id: String) {
        let Some(ticket) = self.start(Section::Detail, |_, view| {
            view.detail = Some(PhotoDetailView {
                tile: view::find_tile(&*view, &id),
                id: id.clone(),
                loading: true,
                problem: None,
                info: None,
            });
            true
        }) else {
            return;
        };
        self.fetch(
            ticket,
            library_item_path(&id),
            |_, view, result: Result<PhotoDetail, _>| {
                let Some(open) = view.detail.as_mut() else {
                    return;
                };
                open.loading = false;
                match result {
                    Ok(detail) => {
                        open.tile = Some(PhotoTile::from(&detail.asset));
                        open.info = Some(PhotoInfo::from(&detail));
                        open.problem = None;
                    }
                    Err(failure) => open.problem = Some(failure),
                }
            },
        );
    }

    /// 写操作：成了的在格子上改，部分失败逐条带出；相册变了的接着重取相册，恢复的接着重取时间线。
    pub(super) fn edit(self: &Arc<Self>, edit: PhotoEdit) -> Result<(), CoreError> {
        let empty = match &edit {
            PhotoEdit::CreateAlbum { title, .. } => title.trim().is_empty(),
            PhotoEdit::ShareAlbum { people, .. } => people.is_empty(),
            other => other.ids().is_empty(),
        };
        if empty {
            return Err(invalid("an edit needs a title, people or items"));
        }
        let Some(ticket) = self.start(Section::Edit, |_, view| {
            view.editing += 1;
            true
        }) else {
            return Ok(());
        };
        let api = MediaApi::connect(&self.home);
        let path = api::core_path(PATH_LIBRARY_ACTIONS.to_owned(), ticket.source.as_deref());
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let result = match api {
                Ok(api) => api.post::<LibraryActionResult>(&path, &edit.wire()).await,
                Err(failure) => Err(failure),
            };
            let succeeded = result.is_ok();
            let applied = link.apply(&ticket, |_, view| finish_edit(view, &edit, result));
            if applied && succeeded {
                link.after_edit(&edit);
            }
        });
        Ok(())
    }

    fn after_edit(self: &Arc<Self>, edit: &PhotoEdit) {
        let open = self
            .view
            .borrow()
            .album
            .as_ref()
            .map(|album| album.id.clone());
        match edit {
            PhotoEdit::Favorite { .. } => {}
            PhotoEdit::Restore { .. } => {
                self.timeline();
                self.reload_open(|_| true);
                self.albums();
            }
            PhotoEdit::AddToAlbum { album, .. } | PhotoEdit::ShareAlbum { album, .. }
                if open.as_deref() == Some(album) =>
            {
                self.albums();
                self.album(Some(album.clone()), Page::Reload);
            }
            _ => self.albums(),
        }
    }
}

fn finish_edit(
    view: &mut PhotosViewModel,
    edit: &PhotoEdit,
    result: Result<LibraryActionResult, MediaFailure>,
) {
    view.editing = view.editing.saturating_sub(1);
    match result {
        Ok(result) => view::apply_edit(view, edit, &result),
        Err(failure) => {
            view.last_edit = Some(PhotoEditOutcome {
                kind: edit.kind(),
                done: Vec::new(),
                changed: 0,
                failed: Vec::new(),
                album: None,
                problem: Some(failure),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_are_checked_before_they_go_into_a_path() {
        assert!(is_month("2026-11"));
        assert!(!is_month("2026-1"));
        assert!(!is_month("../../x"));
        assert!(!is_month("2026/11"));
    }
}
