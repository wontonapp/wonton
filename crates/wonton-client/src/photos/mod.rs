//! 原生相册页（ADR-0113 第 9 节）：时间线、相册、搜索、人物、大图页、本人的写操作和能共享给的家人。
//!
//! 请求走核心的 `/_wonton/api/media/library/…`，核心按设备认人、转给相册插件；客户端核不直讲任何具体插件。
//! 请求和失败归类用媒体那一套（[`MediaApi`]），字节走媒体的本机回环（[`MediaLink::loopback`]）：这里只出地址，
//! 字节不经绑定层，`Range` 由回环原样转。
//!
//! 游标、按日分组和选择集在这里，壳只画。每一块（时间线、某个月、相册、搜索……）各有自己的票号：后发的请求
//! 作废先发的，关掉的那块在路上的请求回来也不写；换了相册插件（`source`），之前发出的一律作废。换服务器整个
//! 作用域重建，旧作用域的任务等停下了才换（[`crate::scope`]），旧服务器的回包落不进新服务器的视图。

mod api;
mod loads;
mod share;
mod subscription;
mod view;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use serde::de::DeserializeOwned;
use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_protocol::media::MediaFailure;
use wonton_protocol::media::library::{library_content_path, person_thumbnail_path};
use wonton_protocol::media::library_view::{PhotoRendition, PhotosAction, PhotosViewModel};
use wonton_protocol::{CoreError, HomeStatus, HomeViewModel};

use crate::home::HomeLink;
use crate::media_link::{MediaApi, MediaLink};
use crate::scope::reuse;

pub use subscription::PhotosSubscription;

/// 假数据入口照真核心的规则改视图：选中、写操作的结果、找格子和相册。
#[cfg(feature = "ui-fake")]
pub(crate) use view::{apply_edit, find_album, find_tile, select};

pub(crate) struct PhotosLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    media: Arc<MediaLink>,
    view: watch::Sender<PhotosViewModel>,
    state: Mutex<State>,
}

/// 视图里不给壳看的那部分：游标、票号、是不是开着。和视图一起改，锁着它再写视图。
#[derive(Default)]
struct State {
    /// 换相册插件加一：之前发出的请求都作废。
    generation: u64,
    /// 壳选的相册插件；空着由核心按默认挑。
    source: Option<String>,
    /// 相册页开过：重连后自己补取。
    open: bool,
    utc_offset_minutes: i32,
    next_ticket: u64,
    tickets: HashMap<Section, u64>,
    /// 取过的月的下一页游标；`None` 是最后一页。
    month_cursors: HashMap<String, Option<String>>,
    album_cursor: Option<String>,
    search_cursor: Option<String>,
}

/// 各自作废的一块。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Section {
    Features,
    Timeline,
    Albums,
    People,
    ShareTargets,
    Month(String),
    Album,
    Search,
    Detail,
    /// 写操作互不作废，只看相册插件换没换。
    Edit,
}

/// 一次请求的凭据：发出时记下的代、块和序号，回来时对得上才写。
struct Ticket {
    generation: u64,
    section: Section,
    seq: u64,
    source: Option<String>,
}

impl PhotosLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        media: Arc<MediaLink>,
        view: Option<watch::Sender<PhotosViewModel>>,
    ) -> Arc<Self> {
        let link = Arc::new(Self {
            runtime,
            home,
            media,
            view: reuse(view, PhotosViewModel::default()),
            state: Mutex::default(),
        });
        let receiver = link.home.subscribe();
        link.runtime
            .spawn(follow_home(Arc::downgrade(&link), receiver));
        link
    }

    pub(crate) fn sender(&self) -> watch::Sender<PhotosViewModel> {
        self.view.clone()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: PhotosAction) -> Result<(), CoreError> {
        match action {
            PhotosAction::Open {
                utc_offset_minutes,
                source,
            } => {
                self.open(utc_offset_minutes, source);
                self.refresh_lists();
            }
            PhotosAction::Refresh => self.refresh_all(),
            PhotosAction::OpenMonth { month } => self.month(month, loads::Page::First)?,
            PhotosAction::LoadMoreMonth { month } => self.month(month, loads::Page::More)?,
            PhotosAction::OpenAlbum { id } => self.album(Some(id), loads::Page::First),
            PhotosAction::LoadMoreAlbum => self.album(None, loads::Page::More),
            PhotosAction::CloseAlbum => self.close(Section::Album, |view| view.album = None),
            PhotosAction::Search { query } => self.search(Some(query), loads::Page::First)?,
            PhotosAction::LoadMoreSearch => self.search(None, loads::Page::More)?,
            PhotosAction::ClearSearch => self.close(Section::Search, |view| view.search = None),
            PhotosAction::OpenDetail { id } => self.detail(id),
            PhotosAction::CloseDetail => self.close(Section::Detail, |view| view.detail = None),
            PhotosAction::Select { ids, selected } => self.modify(|view| {
                view::select(&mut view.selected, ids, selected);
            }),
            PhotosAction::ClearSelection => self.modify(|view| view.selected.clear()),
            PhotosAction::Edit { edit } => self.edit(edit)?,
            PhotosAction::DismissEditOutcome => self.modify(|view| view.last_edit = None),
        }
        Ok(())
    }

    /// 记下时差和相册插件；插件换了，已取的、在路上的都作废，视图从头来。
    fn open(&self, utc_offset_minutes: i32, source: Option<String>) {
        let mut state = self.lock();
        state.open = true;
        state.utc_offset_minutes = utc_offset_minutes;
        if state.source == source {
            return;
        }
        *state = State {
            generation: state.generation + 1,
            source,
            open: true,
            utc_offset_minutes,
            next_ticket: state.next_ticket,
            ..State::default()
        };
        self.view.send_replace(PhotosViewModel::default());
    }

    /// 在锁里开一块的请求：`begin` 回假就不发（比如没有下一页、正在取），回真就发一张新票，旧票作废。
    fn start(
        &self,
        section: Section,
        begin: impl FnOnce(&mut State, &mut PhotosViewModel) -> bool,
    ) -> Option<Ticket> {
        let mut state = self.lock();
        let mut go = false;
        self.view.send_if_modified(|view| {
            go = begin(&mut state, view);
            go
        });
        if !go {
            return None;
        }
        state.next_ticket += 1;
        let seq = state.next_ticket;
        state.tickets.insert(section.clone(), seq);
        Some(Ticket {
            generation: state.generation,
            section,
            seq,
            source: state.source.clone(),
        })
    }

    /// 票还有效才写：同一块没有更新的请求、相册插件也没换。
    fn apply(&self, ticket: &Ticket, f: impl FnOnce(&mut State, &mut PhotosViewModel)) -> bool {
        let mut state = self.lock();
        let current = state.generation == ticket.generation
            && (ticket.section == Section::Edit
                || state.tickets.get(&ticket.section) == Some(&ticket.seq));
        if current {
            self.view.send_modify(|view| f(&mut state, view));
        }
        current
    }

    fn modify(&self, f: impl FnOnce(&mut PhotosViewModel)) {
        let _state = self.lock();
        self.view.send_modify(f);
    }

    /// 关掉一块：在路上的请求作废，回来也不写。
    fn close(&self, section: Section, f: impl FnOnce(&mut PhotosViewModel)) {
        self.start(section, |_, view| {
            f(view);
            true
        });
    }

    /// 发一个 `GET`（提供方路径），回来时票还有效就交给 `done` 写进视图。
    fn fetch<T, F>(self: &Arc<Self>, ticket: Ticket, path: String, done: F)
    where
        T: DeserializeOwned + Send + 'static,
        F: FnOnce(&mut State, &mut PhotosViewModel, Result<T, MediaFailure>) + Send + 'static,
    {
        let api = MediaApi::connect(&self.home);
        let path = api::core_path(path, ticket.source.as_deref());
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let result = match api {
                Ok(api) => api.get(&path).await,
                Err(failure) => Err(failure),
            };
            link.apply(&ticket, |state, view| done(state, view, result));
        });
    }

    /// 一条的这一档字节的本机回环地址。
    pub(crate) async fn url(
        &self,
        id: &str,
        rendition: PhotoRendition,
    ) -> Result<String, MediaFailure> {
        self.loopback(library_content_path(id, &rendition.wire()))
            .await
    }

    /// 一个人物的头像的本机回环地址。
    pub(crate) async fn person_url(&self, id: &str) -> Result<String, MediaFailure> {
        self.loopback(person_thumbnail_path(id)).await
    }

    async fn loopback(&self, provider_path: String) -> Result<String, MediaFailure> {
        let source = self.lock().source.clone();
        let prefix = self.media.loopback().await?;
        let path = api::with_query(provider_path, &[("source", source.as_deref())]);
        Ok(format!("{prefix}{path}"))
    }

    /// 重新连上：开过相册页就补取。
    fn reconnected(self: &Arc<Self>) {
        if self.lock().open {
            self.recover();
        }
    }
}

/// 每次连上就补取一次：断线期间没取成的那几块自己跟上，不用壳再点。
async fn follow_home(link: Weak<PhotosLink>, mut home: watch::Receiver<HomeViewModel>) {
    let mut was_connected = false;
    loop {
        let connected = {
            let view = home.borrow_and_update();
            view.status == HomeStatus::Connected && view.proxy.is_some()
        };
        if connected && !was_connected {
            let Some(link) = link.upgrade() else {
                return;
            };
            link.reconnected();
        }
        was_connected = connected;
        if home.changed().await.is_err() || link.strong_count() == 0 {
            return;
        }
    }
}
