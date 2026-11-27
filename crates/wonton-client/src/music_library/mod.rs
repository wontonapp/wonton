//! 原生音乐页的曲库浏览（ADR-0113 第 2 节“读路径的例外”）：经插件网页的代理直讲 OpenSubsonic，
//! 一页一个订阅，状态都在这里，壳只画 [`MusicPageView`]。出声和队列不在这里，走播放会话。
//!
//! 页的状态：第一次取是 `Loading`，取到是 `Ready`，一条都没取到才是整页 `Failed`；已有内容时再失败（翻页、
//! 刷新、换搜索词）只在 `Ready` 里挂一条轻提示，内容留着。每次重取或换词都换一个代号，旧请求回来时代号对不上就丢掉，
//! 先发的慢请求不会盖掉后发的。

mod api;
mod covers;
mod fetch;
mod live;
mod map;
mod patch;
mod playable;
#[cfg(test)]
mod playable_tests;
mod star;
mod wire;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_protocol::music::{
    MUSIC_SEARCH_DEBOUNCE_MS, MusicAction, MusicFailure, MusicLyrics, MusicPage, MusicPageAction,
    MusicPageState, MusicPageView, MusicProblem,
};

pub(crate) use api::Endpoint;
use api::Failure;
pub(crate) use live::LiveLibraries;
pub(crate) use playable::playable;

/// 找到一台曲库。生产里是插件列表加本地代理（[`LiveLibraries`]），测试里是本机的假曲库。
pub(crate) trait Libraries: Send + Sync {
    /// 插件网页的入口和经本地代理的 HTTP 客户端；连不上家里、插件没有网页时回失败类别。
    fn endpoint(&self, plugin_id: &str) -> Result<Endpoint, MusicProblem>;
    /// 插件是不是在跑：请求失败时据此分“插件没就绪”和“曲库出错”。
    fn running(&self, plugin_id: &str) -> bool;
}

pub(crate) struct MusicLink {
    handle: Handle,
    libraries: Arc<dyn Libraries>,
    covers: covers::Covers,
    pages: Mutex<HashMap<u64, Arc<Page>>>,
    next_id: AtomicU64,
    stars: star::Stars,
}

/// 一个开着的页。
pub(crate) struct Page {
    id: u64,
    plugin_id: String,
    page: MusicPage,
    view: watch::Sender<MusicPageView>,
    /// 每次重取、换搜索词加一；请求落地时对不上就丢掉。
    generation: AtomicU64,
    /// 搜索页当前的词。
    query: Mutex<String>,
}

/// 一次取怎么落地。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fill {
    /// 第一次取或重取：换掉内容。
    Replace,
    /// 下一页：接在后面。
    Append,
}

impl MusicLink {
    pub(crate) fn new(
        handle: Handle,
        libraries: Arc<dyn Libraries>,
        cache_dir: std::path::PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            handle,
            libraries,
            covers: covers::Covers::new(cache_dir),
            pages: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            stars: star::Stars::default(),
        })
    }

    /// 开一页并开始取。
    pub(crate) fn open(self: &Arc<Self>, plugin_id: String, page: MusicPage) -> Arc<Page> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = Arc::new(Page {
            id,
            plugin_id,
            view: watch::Sender::new(MusicPageView {
                page: page.clone(),
                state: MusicPageState::Loading,
            }),
            page,
            generation: AtomicU64::new(0),
            query: Mutex::new(String::new()),
        });
        lock(&self.pages).insert(id, entry.clone());
        self.spawn_load(&entry, Fill::Replace, None);
        entry
    }

    pub(crate) fn close(&self, page: &Page) {
        lock(&self.pages).remove(&page.id);
    }

    pub(crate) fn act(self: &Arc<Self>, page: &Arc<Page>, action: MusicPageAction) {
        match action {
            MusicPageAction::LoadMore => {
                let ready_for_more = matches!(
                    &page.view.borrow().state,
                    MusicPageState::Ready {
                        has_more: true,
                        loading_more: false,
                        ..
                    }
                );
                if ready_for_more {
                    page.view.send_modify(|view| {
                        if let MusicPageState::Ready { loading_more, .. } = &mut view.state {
                            *loading_more = true;
                        }
                    });
                    self.spawn_load(page, Fill::Append, None);
                }
            }
            MusicPageAction::Reload => self.spawn_load(page, Fill::Replace, None),
            MusicPageAction::Query { text } => {
                if *lock(&page.query) == text {
                    return;
                }
                lock(&page.query).clone_from(&text);
                let wait = Duration::from_millis(MUSIC_SEARCH_DEBOUNCE_MS);
                self.spawn_load(page, Fill::Replace, Some(wait));
            }
        }
    }

    fn spawn_load(self: &Arc<Self>, page: &Arc<Page>, fill: Fill, debounce: Option<Duration>) {
        // 翻页不换代号：翻页时来了重取，重取的代号让翻页的结果作废。
        let generation = if fill == Fill::Replace {
            page.generation.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            page.generation.load(Ordering::SeqCst)
        };
        let link = Arc::downgrade(self);
        let page = page.clone();
        self.handle.spawn(async move {
            if let Some(wait) = debounce {
                tokio::time::sleep(wait).await;
                if page.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
            }
            load(link, page, fill, generation).await;
        });
    }

    /// 加星或取消星：开着的页先改；曲库回失败时，等这一项的请求都落地，再改成曲库实际的状态，并把原因交回壳。
    pub(crate) async fn star(&self, plugin_id: &str, action: MusicAction) -> Option<MusicProblem> {
        let MusicAction::Star { target, starred } = action;
        let ticket = self.stars.begin(plugin_id, &target, starred);
        self.patch_pages(plugin_id, |view| patch::star(view, &target, starred));
        let turn = self.stars.turn().await;
        let result = match self.libraries.endpoint(plugin_id) {
            Ok(endpoint) => fetch::set_star(&endpoint, &target, starred)
                .await
                .map_err(|failure| self.problem(plugin_id, failure)),
            Err(problem) => Err(problem),
        };
        drop(turn);
        if let Some(actual) = self.stars.finish(ticket, result.is_ok()) {
            self.patch_pages(plugin_id, |view| patch::star(view, &target, actual));
        }
        result.err()
    }

    fn patch_pages(&self, plugin_id: &str, change: impl Fn(&mut MusicPageView)) {
        let pages: Vec<Arc<Page>> = lock(&self.pages)
            .values()
            .filter(|page| page.plugin_id == plugin_id)
            .cloned()
            .collect();
        for page in pages {
            page.view.send_modify(&change);
        }
    }

    pub(crate) async fn lyrics(
        &self,
        plugin_id: &str,
        song: &str,
    ) -> Result<Option<MusicLyrics>, MusicProblem> {
        let endpoint = self.libraries.endpoint(plugin_id)?;
        fetch::lyrics(&endpoint, song)
            .await
            .map_err(|failure| self.problem(plugin_id, failure))
    }

    /// 封面图片字节；取不到时为空，壳画占位，原因记日志。
    pub(crate) async fn cover(&self, plugin_id: &str, cover: &str, size: u32) -> Option<Vec<u8>> {
        let endpoint = match self.libraries.endpoint(plugin_id) {
            Ok(endpoint) => endpoint,
            Err(problem) => {
                tracing::warn!(?problem, "music cover: no library endpoint");
                return None;
            }
        };
        self.covers.get(&endpoint, plugin_id, cover, size).await
    }

    fn problem(&self, plugin_id: &str, failure: Failure) -> MusicProblem {
        let running = self.libraries.running(plugin_id);
        let (failure, details) = match failure {
            Failure::Transport(details) => (MusicFailure::Transport, details),
            Failure::Status(status) => (
                match status {
                    _ if !running => MusicFailure::PluginNotReady,
                    401 => MusicFailure::NotSignedIn,
                    403 => MusicFailure::PermissionDenied,
                    _ => MusicFailure::Upstream,
                },
                format!("HTTP {status}"),
            ),
            Failure::Subsonic { code, message } => (
                match code {
                    40 | 41 | 44 => MusicFailure::NotSignedIn,
                    50 => MusicFailure::PermissionDenied,
                    70 => MusicFailure::NotFound,
                    0 => MusicFailure::Upstream,
                    other => MusicFailure::Other(other.to_string()),
                },
                format!("Subsonic {code}: {message}"),
            ),
            Failure::Protocol(details) => (MusicFailure::Protocol, details),
        };
        MusicProblem {
            failure,
            details: Some(details),
        }
    }
}

impl Page {
    pub(crate) fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub(crate) fn view(&self) -> MusicPageView {
        self.view.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<MusicPageView> {
        self.view.subscribe()
    }
}

async fn load(link: Weak<MusicLink>, page: Arc<Page>, fill: Fill, generation: u64) {
    let Some(link) = link.upgrade() else { return };
    let offset = match fill {
        Fill::Replace => 0,
        Fill::Append => patch::loaded_len(&page.view.borrow().state),
    };
    let query = lock(&page.query).clone();
    let result = match link.libraries.endpoint(&page.plugin_id) {
        Ok(endpoint) => {
            let fetched = if page.page == MusicPage::Search {
                fetch::search(&endpoint, &query)
                    .await
                    .map(|content| fetch::Loaded {
                        content,
                        has_more: false,
                    })
            } else {
                fetch::page(&endpoint, &page.page, offset).await
            };
            fetched.map_err(|failure| link.problem(&page.plugin_id, failure))
        }
        Err(problem) => Err(problem),
    };
    if page.generation.load(Ordering::SeqCst) != generation {
        return;
    }
    page.view
        .send_modify(|view| patch::land(&mut view.state, fill == Fill::Append, result));
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
