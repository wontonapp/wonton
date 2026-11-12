//! 两台核心交换联系人事件的通道（ADR-0050）：每对服务器一个不加密的房间（`m.room.create` 的 `type` 是
//! [`CONTACT_ROOM_TYPE`]），成员只有两台的核心账号。
//!
//! - 服务器名按字节序较小的一方在配对成立后建房、邀请对方的核心账号；邀请发不过去（对方离线）就每分钟再试。
//! - 另一方只接受已配对服务器的核心账号发来、带这个类型、而且对方的名字更小的邀请，接受后记下房间。对方重装后
//!   再邀请时换成新房间。邀请先落盘再加入：增量同步不会再带上已经收过的邀请，加入失败（联邦还没通）或者这边
//!   还没记下配对（两边配对落表有先后）时，只能靠记下的这条重试。
//! - 收：核心账号长轮询 `/sync`，只要联系人事件和邀请；`next_batch` 落盘，重启接着收。第一次同步（没有
//!   `next_batch`）只收邀请，不处理房间里的旧事件。房间对得上、发送者是那台已配对服务器的核心账号的事件才交给
//!   [`Contacts::receive`]。
//! - 发：房间还没建好、对方的核心还没加入时报对方连不上，不发进一个对方收不到的房间。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use wonton_protocol::{
    CONTACT_EVENT, CONTACT_ROOM_TYPE, CORE_LOCALPART, ContactError, ContactEvent, PeerStatus,
};

use super::store::StoreError;
use super::{ContactFailure, Contacts, PeerLink, PeerReach};
use crate::matrix::{Matrix, MatrixError};
use crate::peers::Peers;

/// 长轮询的时长，比 Matrix 接口的超时（30 秒）短。
const SYNC_TIMEOUT_MS: u64 = 20_000;
/// 同步出错后最长隔这么久再试。
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// 隔这么久检查一遍该建的房间、该补的邀请。
const ROOMS_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct RoomEntry {
    room_id: String,
    /// 本机建的房间：对方的核心账号已经邀请到了。对方建的房间总是真。
    invited: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct LinkState {
    /// 对方的服务器名 → 联系人房间。
    #[serde(default)]
    rooms: BTreeMap<String, RoomEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    since: Option<String>,
    /// 收到了、还没加入的联系人房间邀请：对方的服务器名 → 房间。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    joins: BTreeMap<String, String>,
}

/// 配对表里联系人通道用到的部分，测试里换成内存里的名单。
pub trait PairedServers: Send + Sync {
    fn paired_names(&self) -> watch::Receiver<BTreeSet<String>>;
    /// 已配对时是转发组件给的状态，没配对时为空。
    fn contact_reach(&self, server_name: &str) -> Option<PeerStatus>;
}

impl PairedServers for Peers {
    fn paired_names(&self) -> watch::Receiver<BTreeSet<String>> {
        Peers::paired_names(self)
    }

    fn contact_reach(&self, server_name: &str) -> Option<PeerStatus> {
        Peers::contact_reach(self, server_name)
    }
}

pub struct MatrixLink {
    matrix: Arc<Matrix>,
    peers: Arc<dyn PairedServers>,
    path: PathBuf,
    state: Mutex<LinkState>,
    contacts: OnceLock<Weak<Contacts>>,
    /// 最近一次没建成、没邀请到的原因，按对方的服务器名：发事件时房间还没好，照这个报，不一律说对方连不上。
    room_failures: Mutex<BTreeMap<String, ContactFailure>>,
}

fn core_user(server: &str) -> String {
    format!("@{CORE_LOCALPART}:{server}")
}

fn upstream(error: &MatrixError) -> ContactFailure {
    ContactFailure::new(ContactError::Upstream, error.to_string())
}

impl MatrixLink {
    pub fn open(
        path: PathBuf,
        matrix: Arc<Matrix>,
        peers: Arc<dyn PairedServers>,
    ) -> Result<Arc<Self>, StoreError> {
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| StoreError::Corrupt {
                path: path.clone(),
                message: e.to_string(),
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => LinkState::default(),
            Err(source) => return Err(StoreError::Io { path, source }),
        };
        Ok(Arc::new(Self {
            matrix,
            peers,
            path,
            state: Mutex::new(state),
            contacts: OnceLock::new(),
            room_failures: Mutex::default(),
        }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LinkState> {
        self.state.lock().expect("联系人房间表锁中毒")
    }

    fn update(&self, change: impl FnOnce(&mut LinkState)) {
        let mut state = self.lock();
        change(&mut state);
        let bytes = serde_json::to_vec_pretty(&*state).expect("房间表总能序列化");
        let tmp = self.path.with_extension("json.tmp");
        let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(e) = written {
            tracing::error!(path = %self.path.display(), error = %e, "联系人房间表写不进去");
        }
    }

    /// 起后台任务：建房、补邀请、收事件。
    pub fn start(self: &Arc<Self>, contacts: Weak<Contacts>) {
        if self.contacts.set(contacts).is_err() {
            return;
        }
        let rooms = Arc::downgrade(self);
        tokio::spawn(async move { keep_rooms(rooms).await });
        let sync = Arc::downgrade(self);
        tokio::spawn(async move { sync_loop(sync).await });
    }

    fn own_server(&self) -> &str {
        self.matrix.server_name()
    }

    /// 本机名字更小的已配对服务器：建房、补邀请。有没办成的返回 `false`。
    async fn ensure_rooms(&self) -> bool {
        let mut done = true;
        let paired: Vec<String> = self.peers.paired_names().borrow().iter().cloned().collect();
        for server in paired
            .into_iter()
            .filter(|s| self.own_server() < s.as_str())
        {
            let entry = self.lock().rooms.get(&server).cloned();
            let session = self.matrix.core_session();
            let room = match entry {
                Some(entry) if entry.invited => continue,
                Some(entry) => entry.room_id,
                None => match session.create_contact_room().await {
                    Ok(room) => {
                        self.update(|state| {
                            state.rooms.insert(
                                server.clone(),
                                RoomEntry {
                                    room_id: room.clone(),
                                    invited: false,
                                },
                            );
                        });
                        room
                    }
                    Err(e) => {
                        tracing::warn!(%server, error = %e, "建联系人房间失败，稍后再试");
                        // 建房只碰本机的 Tuwunel：不是对方连不上。
                        let failure = ContactFailure::new(
                            ContactError::Upstream,
                            format!("建和 {server} 的联系人房间失败：{e}"),
                        );
                        self.note_failure(&server, Some(failure));
                        done = false;
                        continue;
                    }
                },
            };
            self.note_failure(&server, None);
            match session.invite(&room, &core_user(&server)).await {
                Ok(()) => {
                    self.update(|state| {
                        if let Some(entry) = state.rooms.get_mut(&server) {
                            entry.invited = true;
                        }
                    });
                }
                Err(e) => {
                    tracing::info!(%server, error = %e, "联系人房间的邀请没发到对方，稍后再试");
                    done = false;
                }
            }
        }
        done
    }

    fn note_failure(&self, server: &str, failure: Option<ContactFailure>) {
        let mut failures = self.room_failures.lock().expect("联系人房间失败表锁中毒");
        match failure {
            Some(failure) => failures.insert(server.to_owned(), failure),
            None => failures.remove(server),
        };
    }

    /// 加入记下的邀请（[`LinkState::joins`]）。已配对的服务器有没加入成的返回 `false`。
    async fn join_invited(&self) -> bool {
        let joins = self.lock().joins.clone();
        let mut done = true;
        for (server, room) in joins {
            if self.peers.contact_reach(&server).is_none() {
                continue;
            }
            match self.matrix.core_session().join_via(&room, &server).await {
                Ok(()) => {
                    tracing::info!(%room, %server, "加入了联系人房间");
                    self.update(|state| {
                        if state.joins.get(&server) == Some(&room) {
                            state.joins.remove(&server);
                        }
                        state.rooms.insert(
                            server,
                            RoomEntry {
                                room_id: room,
                                invited: true,
                            },
                        );
                    });
                }
                Err(e) => {
                    tracing::warn!(%room, %server, error = %e, "加入联系人房间失败，稍后再试");
                    done = false;
                }
            }
        }
        done
    }

    /// 一次同步：先收邀请，再收事件，处理完才记下 `next_batch`。
    async fn sync_once(&self) -> Result<(), MatrixError> {
        let since = self.lock().since.clone();
        let filter = json!({
            "presence": {"types": []},
            "account_data": {"types": []},
            "room": {
                "timeline": {"types": [CONTACT_EVENT], "limit": 100},
                "state": {"types": ["m.room.create", "m.room.member"]},
                "ephemeral": {"types": []},
                "account_data": {"types": []},
            },
        });
        let value = self
            .matrix
            .core_session()
            .sync(since.as_deref(), &filter, SYNC_TIMEOUT_MS)
            .await?;
        if let Some(invites) = value["rooms"]["invite"].as_object() {
            for (room, invite) in invites {
                self.consider_invite(room, invite);
            }
            self.join_invited().await;
        }
        if since.is_some()
            && let Some(joined) = value["rooms"]["join"].as_object()
        {
            for (room, joined) in joined {
                self.deliver(room, joined).await;
            }
        }
        if let Some(next) = value["next_batch"].as_str() {
            self.update(|state| state.since = Some(next.to_owned()));
        }
        Ok(())
    }

    /// 记下核心账号发来的联系人房间邀请，之后由 [`Self::join_invited`] 加入。是不是已配对到加入时才看。
    fn consider_invite(&self, room: &str, invite: &Value) {
        let events = invite["invite_state"]["events"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let is_contact_room = events.iter().any(|event| {
            event["type"] == "m.room.create" && event["content"]["type"] == CONTACT_ROOM_TYPE
        });
        let me = self.matrix.admin_user_id();
        let inviter = events.iter().find_map(|event| {
            (event["type"] == "m.room.member" && event["state_key"] == me.as_str())
                .then(|| event["sender"].as_str())
                .flatten()
        });
        let Some(server) = inviter.and_then(|inviter| {
            let server = super::server_of(inviter)?;
            (inviter == core_user(server)).then(|| server.to_owned())
        }) else {
            return;
        };
        if !is_contact_room || server.as_str() >= self.own_server() {
            tracing::info!(%room, %server, "不是该由对方建的联系人房间，不接受");
            return;
        }
        self.update(|state| {
            state.joins.insert(server, room.to_owned());
        });
    }

    async fn deliver(&self, room: &str, joined: &Value) {
        let server = self
            .lock()
            .rooms
            .iter()
            .find(|(_, entry)| entry.room_id == room)
            .map(|(server, _)| server.clone());
        let Some(server) = server else {
            return;
        };
        if self.peers.contact_reach(&server).is_none() {
            return;
        }
        let Some(contacts) = self.contacts.get().and_then(Weak::upgrade) else {
            return;
        };
        let events = joined["timeline"]["events"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for event in events {
            if event["type"] != CONTACT_EVENT || event["sender"] != core_user(&server).as_str() {
                continue;
            }
            match serde_json::from_value::<ContactEvent>(event["content"].clone()) {
                Ok(contact) => contacts.receive(&server, contact).await,
                Err(e) => tracing::warn!(%server, error = %e, "读不懂对方发来的联系人事件"),
            }
        }
    }

    async fn send_event(&self, server: &str, event: ContactEvent) -> Result<(), ContactFailure> {
        let unreachable = |why: &str| {
            ContactFailure::new(
                ContactError::PeerUnreachable,
                format!("和 {server} 的联系人房间{why}"),
            )
        };
        let room = self
            .lock()
            .rooms
            .get(server)
            .map(|entry| entry.room_id.clone());
        let Some(room) = room else {
            let failure = self
                .room_failures
                .lock()
                .expect("联系人房间失败表锁中毒")
                .get(server)
                .cloned();
            return Err(failure.unwrap_or_else(|| unreachable("还没建好")));
        };
        let session = self.matrix.core_session();
        let joined = session
            .membership(&room, &core_user(server))
            .await
            .map_err(|e| upstream(&e))?;
        if joined.as_deref() != Some("join") {
            return Err(unreachable("里对方的核心还没加入"));
        }
        let content = serde_json::to_value(&event).expect("联系人事件总能序列化");
        session
            .send_event(
                &room,
                CONTACT_EVENT,
                &content,
                &crate::secret::random_token(),
            )
            .await
            .map(|_| ())
            .map_err(|e| upstream(&e))
    }
}

impl PeerLink for MatrixLink {
    fn reach(&self, server_name: &str) -> PeerReach {
        match self.peers.contact_reach(server_name) {
            Some(status) => PeerReach::Paired(status),
            None => PeerReach::NotPaired,
        }
    }

    fn send(
        &self,
        server_name: &str,
        event: ContactEvent,
    ) -> BoxFuture<'_, Result<(), ContactFailure>> {
        let server = server_name.to_owned();
        Box::pin(async move { self.send_event(&server, event).await })
    }
}

async fn keep_rooms(link: Weak<MatrixLink>) {
    let Some(mut names) = link.upgrade().map(|link| link.peers.paired_names()) else {
        return;
    };
    // 刚配对时对方的联邦常常还没通，第一次邀请、加入会失败；没办成就几秒后再试，不让刚配对就加联系人的人等满一轮。
    let mut retry = Duration::from_secs(2);
    loop {
        let Some(strong) = link.upgrade() else {
            return;
        };
        let rooms_done = strong.ensure_rooms().await;
        let joins_done = strong.join_invited().await;
        let wait = if rooms_done && joins_done {
            retry = Duration::from_secs(2);
            ROOMS_INTERVAL
        } else {
            let wait = retry;
            retry = (retry * 2).min(ROOMS_INTERVAL);
            wait
        };
        drop(strong);
        tokio::select! {
            changed = names.changed() => if changed.is_err() { return },
            () = tokio::time::sleep(wait) => {}
        }
    }
}

async fn sync_loop(link: Weak<MatrixLink>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let Some(strong) = link.upgrade() else {
            return;
        };
        match strong.sync_once().await {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(e) => {
                tracing::warn!(error = %e, "联系人房间同步失败，稍后再试");
                drop(strong);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

#[cfg(test)]
#[path = "matrix_link_tests.rs"]
mod tests;
