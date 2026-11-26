//! 代理 TUN 的假地址（198.18.0.0/15）是否排除出选路：设备级的设置文件，默认关。
//!
//! 打开后绑定的 endpoint 装上 [`wonton_tunnel::PreferRealPaths`]，真直连和中继都排在假地址前面；关着不装，
//! iroh 的默认选路一点不动。读不了时当关并记日志：选路差一点不该挡住 App 起来。

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use wonton_l10n::l10n;
use wonton_protocol::CoreError;

use super::HomeLink;
use crate::home::write_private;
use crate::l10n::{CATALOG, language};

const SETTINGS_FILE: &str = "path-settings.json";

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    exclude_fake_ip: bool,
}

/// 开关是不是开着；文件不在或是空的都算关。
pub(crate) fn exclude_fake_ip(root: &Path) -> bool {
    let text = match fs::read_to_string(root.join(SETTINGS_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(error) => {
            tracing::warn!("failed to read the path settings, keeping the switch off: {error}");
            return false;
        }
    };
    match serde_json::from_str::<Settings>(&text) {
        Ok(settings) => settings.exclude_fake_ip,
        Err(error) => {
            tracing::warn!("failed to parse the path settings, keeping the switch off: {error}");
            false
        }
    }
}

/// 写开关；写不了时返回 `Storage`，设置不变。
pub(crate) fn set_exclude_fake_ip(root: &Path, enabled: bool) -> Result<(), CoreError> {
    let path = root.join(SETTINGS_FILE);
    let text = serde_json::to_vec(&Settings {
        exclude_fake_ip: enabled,
    })
    .expect("the path settings are plain fields and always serialize");
    write_private(&path, &text).map_err(|error| CoreError::Storage {
        reason: l10n!(
            CATALOG,
            language(),
            "path-settings-write-failed",
            path = path.display().to_string(),
            reason = error.to_string()
        ),
    })
}

impl HomeLink {
    /// 写开关、更新缓存并重建 endpoint；写不了时缓存和文件都不变。
    pub(crate) fn set_fake_ip_exclusion(self: &Arc<Self>, enabled: bool) -> Result<(), CoreError> {
        set_exclude_fake_ip(self.storage.root(), enabled)?;
        self.fake_ip_exclusion.store(enabled, Ordering::SeqCst);
        self.reapply_path_setting();
        Ok(())
    }

    /// 开关变了：整个重建 endpoint，和下一次建连拿到的是同一个选路器。
    pub(crate) fn reapply_path_setting(self: &Arc<Self>) {
        let home = self.clone();
        self.runtime.spawn(async move {
            home.rebuild_endpoint("fake-ip path setting changed").await;
        });
    }
}
