//! 主人打开了映射的插件，存在这台服务器的槽位目录里。

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::home::write_private;

pub(super) const FILE: &str = "port-mappings.json";

#[derive(Serialize, Deserialize)]
struct Saved {
    enabled: BTreeSet<String>,
}

/// 文件读不出来时映射全按关处理并记错误日志：映射默认关，读坏了不能替主人打开端口。
pub(super) fn load(path: &Path) -> BTreeSet<String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return BTreeSet::new(),
        Err(err) => {
            tracing::error!(path = %path.display(), "could not read the port mapping switches: {err}");
            return BTreeSet::new();
        }
    };
    match serde_json::from_slice::<Saved>(&bytes) {
        Ok(saved) => saved.enabled,
        Err(err) => {
            tracing::error!(path = %path.display(), "the port mapping switches are corrupt: {err}");
            BTreeSet::new()
        }
    }
}

pub(super) fn save(path: &Path, enabled: &BTreeSet<String>) -> io::Result<()> {
    let saved = Saved {
        enabled: enabled.clone(),
    };
    write_private(path, &serde_json::to_vec(&saved).map_err(io::Error::other)?)
}
