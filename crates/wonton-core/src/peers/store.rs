//! 配对表本身：内存里的记录、落盘，以及接上隧道的入口。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_tunnel::Allowlist;
use wonton_tunnel::federation::PeerStates;

use super::code;
use super::pin::{OwnCertificate, PinState, prune_expired_pins, window_expired};
use super::record::{
    HOUR_MS, OwnIdentity, OwnerNotices, PeerRecord, PeerRequest, PeerStoreError, Phase,
    RejectedRequest, now_ms,
};
use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

/// 服务器配对：记录、出码、扫码、请求、删配对，以及联邦名单。
pub struct Peers {
    path: PathBuf,
    pub(super) own: OwnIdentity,
    pub(super) tunnel: Tunnel,
    /// 出码方的联邦入站名单：已配对的 endpoint ID。
    pub(super) server_allowlist: Allowlist,
    /// 已配对的服务器名，配对和删除时即时更新。给 Tuwunel 配置（名单和 /etc/hosts）订阅。
    pub(super) names: watch::Sender<BTreeSet<String>>,
    /// 联邦转发组件的状态。还没接转发组件时，已配对的行按“正在确认”处理。
    pub(super) forwarder: Mutex<Option<watch::Receiver<PeerStates>>>,
    /// 公网访问总开关（ADR-0090）：关着时已配对的行报“公网访问已关闭”。
    pub(super) public_access: Mutex<Option<Arc<crate::entry_access::EntryAccess>>>,
    /// 主人在“Wonton”私聊里的提醒。
    pub(super) notices: Option<Arc<dyn OwnerNotices>>,
    /// 主人的显示名，每次握手现取：主人改了资料，下次配对用的就是新名字。
    pub(super) owner_display_name: Arc<dyn Fn() -> String + Send + Sync>,
    /// 主人的语言，说明和提醒按它写（ADR-0043）。
    pub(super) languages: Arc<dyn Fn() -> Languages + Send + Sync>,
    /// 本机证书的来源（ADR-0112）：配对成立后发给对方。
    pub(super) own_certificate: Mutex<Option<OwnCertificate>>,
    /// 测试用：每写一次盘推进一版，等状态变化用。
    #[cfg(test)]
    changes: watch::Sender<u64>,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
pub(crate) struct Inner {
    /// 服务器名 → 记录。
    pub(super) peers: BTreeMap<String, PeerRecord>,
    /// 别的服务器发来的请求，一台服务器最多一条。
    pub(super) requests: Vec<PeerRequest>,
    /// 主人拒绝过的请求。
    pub(super) rejected: Vec<RejectedRequest>,
    /// 本机的请求令牌。和记录一起写盘，权限相同。
    pub(super) token: Option<code::PeerPairToken>,
    /// 收下新请求的时刻，算这一小时的条数。
    pub(super) received: Vec<u64>,
}

impl fmt::Debug for Peers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Peers")
            .field("own", &self.own)
            .field("path", &self.path)
            .finish_non_exhaustive()
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

    pub(super) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("配对表锁中毒")
    }

    /// 整份重写记录：先写临时文件再改名，避免写到一半断电留下损坏的记录（照 `devices.json` 的做法）。
    /// 调用方持着 `self.lock()`，这把锁同时串行化写盘。
    pub(super) fn save(&self, inner: &Inner) -> Result<(), PeerStoreError> {
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
