//! 配对记录的读写：内存里的表和 `core/peers.json` 的形状、落盘。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_tunnel::Allowlist;

use super::{
    HOUR_MS, Inner, OwnIdentity, OwnerNotices, PeerRecord, PeerRequest, Peers, Phase, PinState,
    RejectedRequest, code, now_ms, prune_expired_pins, window_expired,
};
use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

/// 记录写入失败。
#[derive(Debug, thiserror::Error)]
pub enum PeerStoreError {
    #[error("读写配对记录 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("配对记录 {path} 已损坏：{message}")]
    Corrupt { path: PathBuf, message: String },
}

/// `core/peers.json` 的形状。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub peers: BTreeMap<String, PeerRecord>,
    #[serde(default)]
    pub requests: Vec<PeerRequest>,
    #[serde(default)]
    pub rejected: Vec<RejectedRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<code::PeerPairToken>,
    /// 收下新请求的时刻，重启后照样算这一小时的条数。
    #[serde(default)]
    pub received: Vec<u64>,
}

fn read_store(path: &Path) -> Result<Stored, PeerStoreError> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Stored>(&text).map_err(|e| PeerStoreError::Corrupt {
            path: path.to_owned(),
            message: e.to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stored::default()),
        Err(source) => Err(PeerStoreError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

impl Peers {
    /// 读取记录；文件不在时从空表开始，顺手发一张码。
    ///
    /// 上次没处理完的请求留下来（重启不再丢），还在等对方同意的记录重启后接着问结果。
    pub fn open(
        path: impl Into<PathBuf>,
        own: OwnIdentity,
        tunnel: Tunnel,
        notices: Option<Arc<dyn OwnerNotices>>,
        owner_display_name: Arc<dyn Fn() -> String + Send + Sync>,
        languages: Arc<dyn Fn() -> Languages + Send + Sync>,
    ) -> Result<Arc<Self>, PeerStoreError> {
        let path = path.into();
        let mut stored = read_store(&path)?;
        stored
            .received
            .retain(|at| at.saturating_add(HOUR_MS) > now_ms());
        let created_token = stored.token.is_none();
        if created_token {
            stored.token = Some(code::issue());
        }
        let version = crate::build_info::version();
        // 老记录没有窗口起点：从这次启动的核心版本补上（ADR-0112 §5b）。离线期间过了宽限期的
        // 旧 pin 顺手丢掉，只留最新一张。
        let mut stored_changed = false;
        for record in stored.peers.values_mut() {
            if let PinState::Pending { since_version } = &mut record.pin_state
                && since_version.is_empty()
            {
                *since_version = crate::build_info::version().to_owned();
                stored_changed = true;
            }
            stored_changed |= prune_expired_pins(&mut record.peer_pin);
            if let PinState::Pending { since_version } = &record.pin_state
                && !since_version.is_empty()
                && window_expired(since_version, version)
            {
                tracing::warn!(
                    server = %record.server_name,
                    since_version,
                    version,
                    "待钉窗口已到期，对端还没升级；握手仍按证书名字放行"
                );
            }
        }
        let paired: BTreeSet<String> = stored
            .peers
            .values()
            .filter(|record| record.phase == Phase::Paired)
            .map(|record| record.server_name.clone())
            .collect();
        let server_allowlist = Allowlist::new(
            stored
                .peers
                .values()
                .filter(|record| record.phase == Phase::Paired)
                .map(|record| record.endpoint_id),
        );
        let waiting: Vec<String> = stored
            .peers
            .values()
            .filter(|record| record.phase == Phase::AwaitingPeer)
            .map(|record| record.server_name.clone())
            .collect();
        let (names, _) = watch::channel(paired);
        let peers = Arc::new(Self {
            inner: Mutex::new(Inner {
                peers: stored.peers,
                requests: stored.requests,
                rejected: stored.rejected,
                token: stored.token,
                received: stored.received,
            }),
            path,
            own,
            tunnel,
            names,
            server_allowlist,
            forwarder: Mutex::new(None),
            public_access: Mutex::new(None),
            notices,
            owner_display_name,
            languages,
            own_certificate: Mutex::new(None),
            #[cfg(test)]
            changes: watch::channel(0).0,
        });
        if created_token || stored_changed {
            let inner = peers.lock();
            if let Err(e) = peers.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
            }
        }
        for name in waiting {
            peers.spawn_poll(&name);
        }
        Ok(peers)
    }

    pub(in crate::peers) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("配对表锁中毒")
    }

    /// 整份重写记录：先写临时文件再改名，避免写到一半断电留下损坏的记录（照 `devices.json` 的做法）。
    /// 调用方持着 `self.lock()`，这把锁同时串行化写盘。
    pub(in crate::peers) fn save(&self, inner: &Inner) -> Result<(), PeerStoreError> {
        let io = |source| PeerStoreError::Io {
            path: self.path.clone(),
            source,
        };
        let stored = Stored {
            peers: inner.peers.clone(),
            requests: inner.requests.clone(),
            rejected: inner.rejected.clone(),
            token: inner.token.clone(),
            received: inner.received.clone(),
        };
        let text = serde_json::to_string_pretty(&stored).map_err(|e| PeerStoreError::Corrupt {
            path: self.path.clone(),
            message: e.to_string(),
        })?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.sync_all().map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)?;
        #[cfg(test)]
        self.changes.send_modify(|version| *version += 1);
        Ok(())
    }

    /// 测试用：等记录或名单变化的版本号，比隔一会儿问一次稳。
    #[cfg(test)]
    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// 测试用：隧道入口（endpoint、设备登记表和设备配对处理方都在里面）。
    #[cfg(test)]
    pub(crate) fn tunnel(&self) -> &Tunnel {
        &self.tunnel
    }

    /// 出码方的联邦入站名单，挂在联邦协议上：名单里的 endpoint ID 才能进联邦。
    pub fn server_allowlist(&self) -> &Allowlist {
        &self.server_allowlist
    }

    /// 已配对的服务器名，配对和删除时即时更新。给 Tuwunel 的配置（联邦名单和 `/etc/hosts`）订阅；
    /// 名单变化后由订阅方改 Tuwunel 的配置并让它重读（管理房间命令，fed-tuwunel-config 的任务）。
    pub fn paired_names(&self) -> watch::Receiver<BTreeSet<String>> {
        self.names.subscribe()
    }
}
