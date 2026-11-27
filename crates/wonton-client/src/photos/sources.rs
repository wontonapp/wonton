//! 用哪个相册插件：先问核心这个人能用哪几家（`GET /sources`），定下来再取其余几块。
//!
//! 挑哪一家的规则在核心（ADR-0113“选哪个提供方”），这里只决定要不要问用户：一家都没有如实说没装；
//! 只有一家或主人定了默认，不带 `source`，由核心照规则挑；几家又没默认、用户也没挑过，让用户挑。
//! 用户挑的那家存在这台服务器的槽位目录里，同一台手机连两台服务器各记各的；那家不在候选里了（卸了、
//! 不给他用了）就当没挑过。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use wonton_protocol::media::choices::{MediaSources, SourcesQuery, sources_path};
use wonton_protocol::media::library_view::{
    PhotoSource, PhotoSourceState, PhotoSources, PhotosViewModel,
};
use wonton_protocol::media::{MediaKind, MediaOperation};

use super::{PhotosLink, Section, State};
use crate::media_link::MediaApi;

const FILE: &str = "photos-source.json";

/// 定下来之后接着做什么。
#[derive(Clone, Copy)]
pub(super) enum After {
    /// 取各块的列表
    Open,
    /// 列表和打开着的那几块都重取
    Refresh,
    /// 列表重取，打开着的只补取上次没取成的
    Recover,
}

/// 用户挑过的那家，存在槽位目录里。
pub(super) struct Picked {
    path: PathBuf,
    current: Mutex<Option<String>>,
}

impl Picked {
    /// 读不出来（没有、坏了）当作没挑过：几家都能用时会再问一次。
    pub(super) fn open(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let current = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    fn get(&self) -> Option<String> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn save(&self, id: &str) {
        let bytes = serde_json::to_vec(id).expect("a plugin id always serializes");
        if let Err(error) = std::fs::write(&self.path, bytes) {
            tracing::warn!(%error, "photo source choice not saved to disk; using it for this process only");
        }
        *self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(id.to_owned());
    }
}

/// 这一次用哪家、要不要问用户。`asked` 是 `Open` 这一次点名的，`picked` 是挑过的。
fn decide(
    candidates: &[PhotoSource],
    asked: Option<&str>,
    picked: Option<&str>,
) -> (PhotoSourceState, Option<String>) {
    if let Some(asked) = asked {
        return (PhotoSourceState::Ready, Some(asked.to_owned()));
    }
    if let Some(picked) = picked.filter(|id| candidates.iter().any(|c| c.id == *id)) {
        return (PhotoSourceState::Ready, Some(picked.to_owned()));
    }
    match candidates {
        [] => (PhotoSourceState::NotInstalled, None),
        [_] => (PhotoSourceState::Ready, None),
        several if several.iter().any(|c| c.is_default) => (PhotoSourceState::Ready, None),
        _ => (PhotoSourceState::Choose, None),
    }
}

impl PhotosLink {
    /// 用户挑了一家：记下来，再问一遍核心（挑的那家可能刚被卸了），定下来就取。
    pub(super) fn choose_source(self: &Arc<Self>, id: String) {
        self.picked.save(&id);
        let open = {
            let mut state = self.lock();
            state.asked = None;
            state.open
        };
        if open {
            self.check_sources(After::Open);
        }
    }

    pub(super) fn check_sources(self: &Arc<Self>, then: After) {
        let Some(ticket) = self.start(Section::Sources, |_, view| {
            view.sources.state = PhotoSourceState::Checking;
            true
        }) else {
            return;
        };
        let api = MediaApi::connect(&self.home);
        let path = sources_path(&SourcesQuery {
            operation: MediaOperation::Library,
            kind: Some(MediaKind::Photo),
        });
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let result = match api {
                Ok(api) => api.get::<MediaSources>(&path).await,
                Err(failure) => Err(failure),
            };
            let picked = link.picked.get();
            let mut ready = false;
            link.apply(&ticket, |state, view| match result {
                Ok(listed) => {
                    let candidates: Vec<PhotoSource> =
                        listed.sources.into_iter().map(PhotoSource::from).collect();
                    let (status, source) =
                        decide(&candidates, state.asked.as_deref(), picked.as_deref());
                    switch(state, view, source.clone());
                    view.sources = PhotoSources {
                        state: status,
                        problem: None,
                        candidates,
                        chosen: source,
                    };
                    ready = status == PhotoSourceState::Ready;
                }
                // 候选不确定就不取其余几块：照旧取下去可能取到用户没挑的那家。
                Err(failure) => {
                    view.sources.state = PhotoSourceState::Failed;
                    view.sources.problem = Some(failure);
                }
            });
            if ready {
                match then {
                    After::Open => link.refresh_lists(),
                    After::Refresh => link.refresh_all(),
                    After::Recover => link.recover(),
                }
            }
        });
    }
}

/// 换了相册插件：已取的、在路上的都作废，视图除了候选从头来。没换就不动。
pub(super) fn switch(state: &mut State, view: &mut PhotosViewModel, source: Option<String>) {
    if state.source == source {
        return;
    }
    let sources = std::mem::take(&mut view.sources);
    *state = State {
        generation: state.generation + 1,
        source,
        asked: state.asked.take(),
        open: state.open,
        utc_offset_minutes: state.utc_offset_minutes,
        next_ticket: state.next_ticket,
        ..State::default()
    };
    *view = PhotosViewModel {
        sources,
        ..PhotosViewModel::default()
    };
}
