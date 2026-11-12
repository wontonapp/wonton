//! 插件网页用摄像头/麦克风的记忆：按插件 ID 记在本机，询问的回答和详情页的开关共用一份。
//! 读坏了按没问过处理：下次请求重新出询问，绝不因此放行。

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::PluginsLink;
use crate::home::write_private;
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;
use wonton_protocol::{CoreError, PluginMediaGrant};

impl PluginsLink {
    /// 网页要摄像头/麦克风时按插件查记着的决定；没问过是 `None`。
    pub(crate) fn media_grant(&self, plugin_id: &str) -> Option<bool> {
        self.media.grant(plugin_id)
    }

    /// `SetPluginMediaPermission`：详情页的开关，`true` 是允许，`false` 是改回没问过。
    pub(crate) fn set_media_permission(
        &self,
        plugin_id: String,
        allow: bool,
    ) -> Result<(), CoreError> {
        if allow {
            self.media.record(&plugin_id, true)
        } else {
            self.media.reset(&plugin_id)
        }
    }

    /// 询问弹窗的回答：按插件记下允许或拒绝（拒绝只在询问里产生）。
    pub(crate) fn record_media_decision(
        &self,
        plugin_id: &str,
        allowed: bool,
    ) -> Result<(), CoreError> {
        self.media.record(plugin_id, allowed)
    }
}

const FILE: &str = "plugin-media.json";

#[derive(Serialize, Deserialize)]
struct Saved {
    grants: BTreeMap<String, bool>,
}

pub(super) fn load(path: &Path) -> BTreeMap<String, bool> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(err) => {
            tracing::error!(path = %path.display(), "could not read the plugin media grants: {err}");
            return BTreeMap::new();
        }
    };
    match serde_json::from_slice::<Saved>(&bytes) {
        Ok(saved) => saved.grants,
        Err(err) => {
            tracing::error!(path = %path.display(), "the plugin media grants are corrupt: {err}");
            BTreeMap::new()
        }
    }
}

fn save(path: &Path, grants: &BTreeMap<String, bool>) -> Result<(), CoreError> {
    let saved = Saved {
        grants: grants.clone(),
    };
    write_private(
        path,
        &serde_json::to_vec(&saved).expect("media grants serialize"),
    )
    .map_err(|err| CoreError::Storage {
        reason: l10n!(
            CATALOG,
            language(),
            "plugin-media-save-failed",
            path = path.display().to_string(),
            reason = err.to_string()
        ),
    })
}

/// 记忆和它的视图：改了就落盘并把 [`PluginMediaGrant`] 列表推给订阅者。
pub(super) struct MediaGrants {
    file: PathBuf,
    grants: Mutex<BTreeMap<String, bool>>,
    view: watch::Sender<wonton_protocol::PluginsViewModel>,
}

impl MediaGrants {
    pub(super) fn new(
        data_dir: &Path,
        view: watch::Sender<wonton_protocol::PluginsViewModel>,
    ) -> Self {
        let file = data_dir.join(FILE);
        let grants = load(&file);
        let media = Self {
            file,
            grants: Mutex::new(grants),
            view,
        };
        media.publish();
        media
    }

    /// 这个插件记着的决定；没问过是 `None`。
    pub(super) fn grant(&self, plugin_id: &str) -> Option<bool> {
        self.grants
            .lock()
            .expect("the media grants lock never panics")
            .get(plugin_id)
            .copied()
    }

    /// 记一个决定（询问的回答：`true` 允许，`false` 拒绝）。存不下就不改，错误原样返回。
    pub(super) fn record(&self, plugin_id: &str, allowed: bool) -> Result<(), CoreError> {
        let mut grants = self
            .grants
            .lock()
            .expect("the media grants lock never panics");
        if grants.get(plugin_id) == Some(&allowed) {
            return Ok(());
        }
        let mut next = grants.clone();
        next.insert(plugin_id.to_owned(), allowed);
        save(&self.file, &next)?;
        *grants = next;
        drop(grants);
        self.publish();
        Ok(())
    }

    /// `SetPluginMediaPermission { allow: false }`：改回没问过（清掉，下次再问）。没问过时错误。
    pub(super) fn reset(&self, plugin_id: &str) -> Result<(), CoreError> {
        let mut grants = self
            .grants
            .lock()
            .expect("the media grants lock never panics");
        if grants.remove(plugin_id).is_none() {
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "plugin-media-not-asked"),
            });
        }
        save(&self.file, &grants)?;
        drop(grants);
        self.publish();
        Ok(())
    }

    fn publish(&self) {
        let grants = self
            .grants
            .lock()
            .expect("the media grants lock never panics");
        let list: Vec<PluginMediaGrant> = grants
            .iter()
            .map(|(plugin_id, allowed)| PluginMediaGrant {
                plugin_id: plugin_id.clone(),
                allowed: *allowed,
            })
            .collect();
        drop(grants);
        self.view.send_if_modified(|view| {
            let changed = view.media_grants != list;
            view.media_grants = list;
            changed
        });
    }
}
