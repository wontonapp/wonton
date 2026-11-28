//! 访客链接（ADR-0090 第 5 节）：一个应用最多 20 条有效链接，默认 4 小时、最长 7 天。
//!
//! 令牌是 128 位随机数的小写 base32，和专用密码一样存在核心数据目录（0600）。链接收回或到期时
//! 置位自己的 watch：入口转发层据此断开已经升级的 WebSocket（和开关关闭同一个形状）。
//! 有效期到点由每条记录自己的定时任务置位，不靠下一个请求发现。

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_protocol::{GUEST_LINKS_MAX, GUEST_TTL_DEFAULT, GUEST_TTL_MAX};

use crate::secret::constant_time_eq;

/// 存放位置（相对核心数据目录），和专用密码一条规则。
pub(crate) const FILE_NAME: &str = "clients/guests.json";

/// 只对当前主机名有效的访客 Cookie（ADR-0090 第 5 节）。`__Host-` 前缀要求 `Secure`、`Path=/`、不带 `Domain`。
pub const COOKIE_NAME: &str = "__Host-wonton_guest";

/// 链接名字的字节上限。
const LABEL_MAX: usize = 128;
/// 落地路径的字节上限，和清单路径规则一致。
const PATH_MAX: usize = 256;
/// “最近一次被用”最多这么久落一次盘：访客请求不用每个都写文件。
const PERSIST_EVERY_MS: u64 = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    id: String,
    label: String,
    token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    created_at_ms: u64,
    expires_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_used_at_ms: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    plugins: BTreeMap<String, Vec<Record>>,
}

#[derive(Debug, thiserror::Error)]
pub enum GuestError {
    #[error("read or write guest link storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("guest link storage is corrupt: {0}")]
    Corrupt(String),
    #[error("guest link was not found")]
    NotFound,
    #[error("guest link label is invalid")]
    InvalidLabel,
    #[error("guest link validity is too long")]
    InvalidTtl,
    #[error("guest link landing path is invalid")]
    InvalidPath,
    #[error("the app already has the maximum number of valid guest links")]
    TooMany,
}

/// 一条链接的只读快照，给列表和取地址用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guest {
    pub id: String,
    pub label: String,
    pub path: Option<String>,
    pub expires_at_ms: u64,
    pub last_used_at_ms: Option<u64>,
}

/// 令牌换到的东西：链接的路径范围和它的撤销信号。
#[derive(Debug)]
pub(crate) struct Authenticated {
    pub id: String,
    pub label: String,
    pub path: Option<String>,
    pub expires_at_ms: u64,
    /// 置位就是收回或过期。
    pub revoke: watch::Receiver<bool>,
}

/// 令牌对不上，或者已经失效。对不上按验证失败计数，过期的不用。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TokenError {
    Wrong,
    Gone,
}

struct Inner {
    file: File,
    /// （插件，链接）→ 撤销信号；链接还在有效期内时在里面。
    guards: BTreeMap<(String, String), watch::Sender<bool>>,
    /// 最近一次落盘的时间。
    persisted_ms: u64,
}

pub struct GuestStore {
    path: PathBuf,
    inner: Mutex<Inner>,
}

impl GuestStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, GuestError> {
        let path = data_dir.as_ref().join(FILE_NAME);
        let mut file: File = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|error| GuestError::Corrupt(error.to_string()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => File::default(),
            Err(error) => return Err(error.into()),
        };
        let now = now_ms();
        let mut guards = BTreeMap::new();
        for (plugin, records) in &mut file.plugins {
            records.retain(|record| record.expires_at_ms > now);
            for record in records {
                let (sender, receiver) = watch::channel(false);
                drop(receiver);
                arm(record.expires_at_ms, sender.clone());
                guards.insert((plugin.clone(), record.id.clone()), sender);
            }
        }
        let store = Self {
            path,
            inner: Mutex::new(Inner {
                file,
                guards,
                persisted_ms: now,
            }),
        };
        store.ensure_mode()?;
        Ok(store)
    }

    /// 这个插件还有效的链接，按新建时间排。
    pub fn list(&self, plugin: &str) -> Vec<Guest> {
        let mut inner = self.lock();
        prune(&mut inner);
        inner
            .file
            .plugins
            .get(plugin)
            .into_iter()
            .flatten()
            .map(view)
            .collect()
    }

    /// 新建一条。`ttl_seconds` 为 0 是默认 4 小时；落地路径不带就停在应用首页。
    pub fn create(
        &self,
        plugin: &str,
        label: &str,
        ttl_seconds: u32,
        path: Option<&str>,
    ) -> Result<Guest, GuestError> {
        if label.trim().is_empty() || label.len() > LABEL_MAX || label.contains(['\n', '\r']) {
            return Err(GuestError::InvalidLabel);
        }
        let ttl = if ttl_seconds == 0 {
            GUEST_TTL_DEFAULT
        } else {
            ttl_seconds
        };
        if ttl > GUEST_TTL_MAX {
            return Err(GuestError::InvalidTtl);
        }
        if let Some(path) = path
            && !valid_path(path)
        {
            return Err(GuestError::InvalidPath);
        }
        let mut inner = self.lock();
        prune(&mut inner);
        let records = inner.file.plugins.entry(plugin.to_owned()).or_default();
        if records.len() >= GUEST_LINKS_MAX {
            return Err(GuestError::TooMany);
        }
        let now = now_ms();
        let record = Record {
            id: next_id(records),
            label: label.to_owned(),
            token: crate::clients::passwords::random_value(),
            path: path.map(str::to_owned),
            created_at_ms: now,
            expires_at_ms: now + u64::from(ttl) * 1000,
            last_used_at_ms: None,
        };
        records.push(record.clone());
        let (sender, receiver) = watch::channel(false);
        drop(receiver);
        arm(record.expires_at_ms, sender.clone());
        inner
            .guards
            .insert((plugin.to_owned(), record.id.clone()), sender);
        self.save(&mut inner)?;
        Ok(view(&record))
    }

    /// 取一条还有效的链接。
    pub fn get(&self, plugin: &str, id: &str) -> Result<Guest, GuestError> {
        let mut inner = self.lock();
        prune(&mut inner);
        find(&inner, plugin, id).map(view)
    }

    /// 一条链接的令牌；列表和取地址拼 URL 用。链接不在时为空。
    pub fn token(&self, plugin: &str, id: &str) -> Option<String> {
        let inner = self.lock();
        inner
            .file
            .plugins
            .get(plugin)
            .and_then(|records| records.iter().find(|record| record.id == id))
            .map(|record| record.token.clone())
    }

    /// 收回：下一个请求就失效，已升级的连接立即断。已经不存在时 [`GuestError::NotFound`]。
    pub fn revoke(&self, plugin: &str, id: &str) -> Result<(), GuestError> {
        let mut inner = self.lock();
        prune(&mut inner);
        let records = inner
            .file
            .plugins
            .get_mut(plugin)
            .ok_or(GuestError::NotFound)?;
        let at = records
            .iter()
            .position(|record| record.id == id)
            .ok_or(GuestError::NotFound)?;
        records.remove(at);
        if let Some(sender) = inner.guards.remove(&(plugin.to_owned(), id.to_owned())) {
            sender.send_replace(true);
        }
        self.save(&mut inner)
    }

    /// 用令牌换到链接。令牌不是这个插件的、或者已经过期的都不行。
    pub(crate) fn authenticate(
        &self,
        plugin: &str,
        token: &str,
    ) -> Result<Authenticated, TokenError> {
        let inner = self.lock();
        let record = inner
            .file
            .plugins
            .get(plugin)
            .into_iter()
            .flatten()
            .find(|record| constant_time_eq(&record.token, token));
        let Some(record) = record else {
            return Err(TokenError::Wrong);
        };
        if record.expires_at_ms <= now_ms() {
            return Err(TokenError::Gone);
        }
        let revoke = inner
            .guards
            .get(&(plugin.to_owned(), record.id.clone()))
            .map(watch::Sender::subscribe)
            .unwrap_or_else(|| watch::channel(false).1);
        Ok(Authenticated {
            id: record.id.clone(),
            label: record.label.clone(),
            path: record.path.clone(),
            expires_at_ms: record.expires_at_ms,
            revoke,
        })
    }

    /// 记一次使用；最多每分钟落一次盘。
    pub(crate) fn note_used(&self, plugin: &str, id: &str) {
        let mut inner = self.lock();
        let now = now_ms();
        if let Some(record) = inner
            .file
            .plugins
            .get_mut(plugin)
            .and_then(|records| records.iter_mut().find(|record| record.id == id))
        {
            record.last_used_at_ms = Some(now);
        }
        if now.saturating_sub(inner.persisted_ms) < PERSIST_EVERY_MS {
            return;
        }
        match self.save(&mut inner) {
            Ok(()) => inner.persisted_ms = now,
            // 记不上只影响“最近用过”的时间，请求照走。
            Err(error) => tracing::error!(%error, "访客链接使用时间写不进盘"),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("guest link lock poisoned")
    }

    fn save(&self, inner: &mut Inner) -> Result<(), GuestError> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| GuestError::Corrupt("guest link path has no parent".to_owned()))?;
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&inner.file)
            .map_err(|error| GuestError::Corrupt(error.to_string()))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        fs::rename(tmp, &self.path)?;
        inner.persisted_ms = now_ms();
        self.ensure_mode()
    }

    fn ensure_mode(&self) -> Result<(), GuestError> {
        if self.path.exists() {
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

/// 到期置位。没有运行时（纯同步测试）时不挂定时任务，靠请求时的有效期判断兜底。
fn arm(expires_at_ms: u64, sender: watch::Sender<bool>) {
    let Some(remaining) = expires_at_ms.checked_sub(now_ms()) else {
        sender.send_replace(true);
        return;
    };
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        tokio::time::sleep(Duration::from_millis(remaining)).await;
        sender.send_replace(true);
    });
}

fn prune(inner: &mut Inner) {
    let now = now_ms();
    let mut gone = Vec::new();
    for (plugin, records) in &mut inner.file.plugins {
        let mut kept = Vec::with_capacity(records.len());
        for record in records.drain(..) {
            if record.expires_at_ms > now {
                kept.push(record);
            } else {
                gone.push((plugin.clone(), record.id));
            }
        }
        *records = kept;
    }
    for key in gone {
        if let Some(sender) = inner.guards.remove(&key) {
            sender.send_replace(true);
        }
    }
}

fn find<'a>(inner: &'a Inner, plugin: &str, id: &str) -> Result<&'a Record, GuestError> {
    inner
        .file
        .plugins
        .get(plugin)
        .and_then(|records| records.iter().find(|record| record.id == id))
        .ok_or(GuestError::NotFound)
}

fn view(record: &Record) -> Guest {
    Guest {
        id: record.id.clone(),
        label: record.label.clone(),
        path: record.path.clone(),
        expires_at_ms: record.expires_at_ms,
        last_used_at_ms: record.last_used_at_ms,
    }
}

fn valid_path(path: &str) -> bool {
    // `\` 也算：浏览器把 `/\evil.test` 当成 `//evil.test`，那就不是同源路径了。
    path.starts_with('/')
        && !path.starts_with("//")
        && path.len() <= PATH_MAX
        && !path
            .bytes()
            .any(|byte| byte < 0x20 || matches!(byte, b' ' | b'\\' | b'#' | b'?'))
}

fn next_id(records: &[Record]) -> String {
    let mut number = records.len() + 1;
    loop {
        let id = format!("g-{number}");
        if !records.iter().any(|record| record.id == id) {
            return id;
        }
        number += 1;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

#[cfg(test)]
#[path = "guests_tests.rs"]
mod guests_tests;
