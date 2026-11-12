//! `wonton-acp-bridge health`：容器健康检查用（规范“插件健康”）。两个文件都在容器本地文件系统：
//! `alive` 由桥的主进程定时写，进程死了文件就不新了；`crypto-degraded` 是“长期解不开”的标记，
//! 主进程发提示时写。`docker restart` 后文件还在——store 丢了这个事实本身就在，新进程到点会
//! 再判一次；重装插件重建容器时才一起清掉。
//!
//! 目录由 `WONTON_ACP_BRIDGE_HEALTH_DIR` 指定，默认 `/tmp`：写的心跳和查的命令是两个进程，靠同一个
//! 环境变量找同一处；同机跑多个测试时各用各的临时目录。

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, bail};

/// 生产用的目录。
const DEFAULT_DIR: &str = "/tmp";
/// 桥主进程的心跳文件名，在目录里。
const ALIVE_FILE: &str = "wonton-acp-bridge.alive";
/// 判定“长期解不开”后写的标记文件名，内容是给主人看的原因。
const DEGRADED_FILE: &str = "wonton-acp-bridge.crypto-degraded";

/// 心跳多久没更新算进程出事。桥每 10 秒摸一次。
const STALE: Duration = Duration::from_secs(90);

/// 心跳和标记文件在哪个目录的哪两个文件。
#[derive(Clone, Debug)]
pub(crate) struct Files {
    alive: PathBuf,
    degraded: PathBuf,
}

impl Files {
    pub(crate) fn in_dir(dir: &Path) -> Self {
        Self {
            alive: dir.join(ALIVE_FILE),
            degraded: dir.join(DEGRADED_FILE),
        }
    }

    pub(crate) fn from_env() -> Self {
        match std::env::var("WONTON_ACP_BRIDGE_HEALTH_DIR") {
            Ok(dir) => Self::in_dir(Path::new(&dir)),
            Err(_) => Self::in_dir(Path::new(DEFAULT_DIR)),
        }
    }

    pub(crate) fn touch_alive(&self) {
        if let Err(error) = std::fs::write(&self.alive, b"") {
            tracing::warn!(error = %error, path = %self.alive.display(), "写不了心跳文件");
        }
    }

    /// 判定“长期解不开”时记下原因（本房间的提示原文），健康检查把它带出去。
    pub(crate) fn mark_degraded(&self, reason: &str) {
        if let Err(error) = std::fs::write(&self.degraded, reason) {
            tracing::warn!(error = %error, path = %self.degraded.display(), "写不了长期解不开标记");
        }
    }

    /// 健康检查：先报“长期解不开”，再看进程心跳。
    pub(crate) fn run(&self) -> Result<()> {
        if let Ok(reason) = std::fs::read_to_string(&self.degraded) {
            bail!("{}", reason.trim());
        }
        let modified = std::fs::metadata(&self.alive)
            .and_then(|metadata| metadata.modified())
            .with_context(|| format!("桥还没起来过（没有 {}）", self.alive.display()))?;
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default();
        if age > STALE {
            bail!("桥 {} 秒没有心跳", age.as_secs());
        }
        Ok(())
    }
}
