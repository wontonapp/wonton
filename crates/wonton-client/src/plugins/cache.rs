//! 已安装插件的列表存在本机，冷启动没网也能显示主屏（ADR-0032）。丢了不算错，连上后会重新取。

use std::path::Path;

use serde::{Deserialize, Serialize};
use wonton_protocol::InstalledPlugins;

#[derive(Serialize, Deserialize)]
struct Cached {
    server_id: String,
    installed: InstalledPlugins,
}

pub(super) fn load(path: &Path, server_id: &str) -> Option<InstalledPlugins> {
    let bytes = std::fs::read(path).ok()?;
    let cached: Cached = serde_json::from_slice(&bytes)
        .inspect_err(|error| tracing::warn!("could not read the cached app list: {error}"))
        .ok()?;
    if cached.server_id != server_id {
        return None;
    }
    let mut installed = cached.installed;
    clear_bot_ids(&mut installed);
    Some(installed)
}

pub(super) async fn save(path: &Path, server_id: String, mut installed: InstalledPlugins) {
    clear_bot_ids(&mut installed);
    let cached = Cached {
        server_id,
        installed,
    };
    let saved = async {
        let bytes = serde_json::to_vec(&cached).map_err(std::io::Error::other)?;
        let partial = path.with_extension("partial");
        tokio::fs::write(&partial, bytes).await?;
        tokio::fs::rename(&partial, path).await
    };
    if let Err(error) = saved.await {
        tracing::warn!("could not save the app list: {error}");
    }
}

fn clear_bot_ids(installed: &mut InstalledPlugins) {
    for plugin in &mut installed.plugins {
        plugin.bot_user_id = None;
        plugin.default_bot_visibility = Some(wonton_protocol::DefaultBotVisibility::Unknown);
    }
}

pub(super) fn forget(path: &Path) {
    if let Err(error) = std::fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("could not remove the previous server's app list: {error}");
    }
}
