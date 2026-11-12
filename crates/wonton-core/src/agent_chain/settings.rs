//! Agent 的设置（ADR-0061 第 10 节）：链的上限和每个使用方的并发，只有主人能改，落盘在核心数据目录。
//!
//! 缺文件等于默认值；坏文件报错，不拿默认值顶替。上限只影响之后新开的链，旧链用“再给一份”。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use wonton_protocol::{ChainLimits, DEFAULT_CONSUMER_AGENT_CONCURRENCY};

use super::ChainLedgerError;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Stored {
    /// 新链的上限；缺失时是默认值。
    #[serde(default)]
    limits: Option<ChainLimits>,
    /// 使用方（插件 ID）→ 同时能开的 `agent_acp` 连接数；没写的是默认值。
    #[serde(default)]
    concurrency: BTreeMap<String, u32>,
}

/// 设置“Agent”一页的存储。所有方法线程安全，写操作串行并落盘。
pub struct AgentSettings {
    path: PathBuf,
    stored: Mutex<Stored>,
}

impl AgentSettings {
    /// 读回设置。文件缺失从默认值开始，坏文件报错。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, ChainLedgerError> {
        let path = path.into();
        let stored = read(&path)?;
        Ok(Self {
            path,
            stored: Mutex::new(stored),
        })
    }

    /// 新链的上限。
    pub fn limits(&self) -> ChainLimits {
        self.lock().limits.unwrap_or_default()
    }

    /// 改上限：写盘成功才生效。
    pub fn set_limits(&self, limits: ChainLimits) -> Result<(), ChainLedgerError> {
        self.change(|stored| stored.limits = Some(limits))
    }

    /// 一个使用方同时能开几条 `agent_acp` 连接：没设过是默认值。
    pub fn concurrency(&self, plugin: &str) -> u32 {
        self.lock()
            .concurrency
            .get(plugin)
            .copied()
            .unwrap_or(DEFAULT_CONSUMER_AGENT_CONCURRENCY)
    }

    /// 改一个使用方的并发上限：写盘成功才生效。
    pub fn set_concurrency(&self, plugin: &str, concurrency: u32) -> Result<(), ChainLedgerError> {
        self.change(|stored| {
            stored.concurrency.insert(plugin.to_owned(), concurrency);
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Stored> {
        self.stored.lock().expect("Agent 设置的锁中毒")
    }

    fn change(&self, change: impl FnOnce(&mut Stored)) -> Result<(), ChainLedgerError> {
        let mut stored = self.lock();
        let mut next = stored.clone();
        change(&mut next);
        write(&self.path, &next)?;
        *stored = next;
        Ok(())
    }
}

fn read(path: &Path) -> Result<Stored, ChainLedgerError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Stored::default()),
        Err(source) => {
            return Err(ChainLedgerError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    serde_json::from_slice(&bytes).map_err(|e| ChainLedgerError::Corrupt {
        path: path.to_owned(),
        message: e.to_string(),
    })
}

/// 私有临时文件、fsync、原子改名、目录 fsync（同 `friends/persistence`）：改名即提交点。
fn write(path: &Path, stored: &Stored) -> Result<(), ChainLedgerError> {
    let io = |source: std::io::Error| ChainLedgerError::Io {
        path: path.to_owned(),
        source,
    };
    let dir = path
        .parent()
        .ok_or_else(|| ChainLedgerError::Invalid(format!("{} 没有目录", path.display())))?;
    fs::create_dir_all(dir).map_err(io)?;
    let body = serde_json::to_vec_pretty(stored)
        .map_err(|e| ChainLedgerError::Serialize(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        file.write_all(&body).map_err(io)?;
        file.sync_all().map_err(io)?;
    }
    fs::rename(&tmp, path).map_err(io)?;
    let dir = fs::File::open(dir).map_err(io)?;
    if let Err(e) = dir.sync_all() {
        tracing::warn!(error = %e, "Agent 设置目录同步失败");
    }
    Ok(())
}
