//! 假 Tuwunel 的状态：用户、房间、令牌、上传和测试用的故障开关，以及服务器自己建的管理房间。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use jsonwebtoken::DecodingKey;
use serde_json::Value;

use super::rooms;

pub const SECRET: &str = "shared-secret";
pub const SERVER: &str = "test";
/// 假服务器建的管理房间和它的别名：管理员注册后由服务器邀请进去（Tuwunel 的 `make_user_admin`）。
pub const ADMIN_ROOM_ID: &str = "!admins:test";
pub const ADMIN_ROOM_ALIAS: &str = "#admins:test";

/// Tuwunel 自己的服务器用户，管理房间的成员。
const SERVER_USER: &str = "conduit";

#[derive(Default)]
pub struct User {
    pub admin: bool,
    pub deactivated: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub avatars: Option<Value>,
    pub direct: Option<Value>,
}

#[derive(Default)]
pub struct Room {
    pub members: BTreeSet<String>,
    pub invited: BTreeSet<String>,
    /// 进过房间又退出（或被撤回邀请、停用账号）的人。
    pub left: BTreeSet<String>,
    pub encrypted: bool,
}

/// 假 Tuwunel 的全部状态，测试可以直接读写。
#[derive(Default)]
pub struct Tuwunel {
    pub users: BTreeMap<String, User>,
    /// 访问令牌 → （用户，设备）。
    pub tokens: BTreeMap<String, (String, String)>,
    pub rooms: BTreeMap<String, Room>,
    /// 管理房间的房间号，`start()` 建好。
    pub admin_room: String,
    pub room_state: BTreeMap<String, rooms::RoomState>,
    /// 上传的文件：（类型，内容）。
    pub media: Vec<(String, Vec<u8>)>,
    pub media_ids: BTreeMap<String, usize>,
    pub media_uploaders: Vec<String>,
    pub media_downloaders: Vec<String>,
    pub registrations: usize,
    /// 为真时建房失败，测试中途失败的处理。
    pub refuse_rooms: bool,
    pub refuse_power_levels: bool,
    pub refuse_room_reads: bool,
    /// 查这些房间的成员时报 500：单间查不到，别的房间照常。
    pub broken_rooms: BTreeSet<String>,
    pub refuse_kicks: bool,
    /// 为真时下一次建房照样建好，但响应体半路断掉（模拟客户端丢掉 createRoom 的响应）。
    pub drop_next_room_create: bool,

    /// 为真时邀请失败。
    pub refuse_invites: bool,
    /// 为真时加入失败（邀请已经发出去了）。
    pub refuse_joins: bool,
    /// 发出的消息：（房间，内容）。
    pub sent: Vec<(String, Value)>,
    /// 为真时拒绝下一条消息（`M_FORBIDDEN`）：核对之后才被踢出去。
    pub refuse_next_send: bool,
    /// 为真时读写资料都失败。
    pub refuse_profile: bool,
    pub refuse_avatar_write: bool,
    pub refuse_logout: bool,
    /// 为真时 `whoami` 报 500：令牌验不了不是因为令牌坏了（Tuwunel 不可用、核心重启中）。
    pub refuse_whoami: bool,
    /// 写过几次显示名。
    pub display_name_writes: usize,
    /// 全局账号数据：（用户，事件类型）→ 内容。
    pub account_data: BTreeMap<(String, String), Value>,
    /// 房间账号数据：（用户，房间，事件类型）→ 内容。
    pub room_account_data: BTreeMap<(String, String, String), Value>,
    next: usize,
}

impl Tuwunel {
    pub(super) fn next(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    /// 给用户登录一台新设备，返回访问令牌。
    pub(super) fn issue(&mut self, user: &str) -> (String, String) {
        let device = self.next("DEVICE");
        let token = self.next("token-");
        self.tokens
            .insert(token.clone(), (user.to_owned(), device.clone()));
        (device, token)
    }

    /// 用户现有的设备。
    pub fn devices(&self, user: &str) -> Vec<String> {
        self.tokens
            .values()
            .filter(|(u, _)| u == user)
            .map(|(_, d)| d.clone())
            .collect()
    }

    pub fn user(&self, user: &str) -> &User {
        &self.users[user]
    }
}

#[derive(Clone)]
pub struct Fake {
    pub(super) state: Arc<Mutex<Tuwunel>>,
    /// 验签核心签的登录令牌用的公钥。
    pub key: Arc<DecodingKey>,
}

impl Fake {
    pub fn state(&self) -> MutexGuard<'_, Tuwunel> {
        self.state.lock().expect("假 Tuwunel 的锁中毒")
    }
}

/// 像 Tuwunel 的 `fresh()` 一样先建管理房间和服务器用户；注册成管理员的账号由 `make_user_admin` 邀请。
pub(super) fn seed_admin_room(state: &mut Tuwunel) {
    state.admin_room = ADMIN_ROOM_ID.to_owned();
    state.rooms.insert(
        ADMIN_ROOM_ID.to_owned(),
        Room {
            members: BTreeSet::from([format!("@{SERVER_USER}:{SERVER}")]),
            ..Room::default()
        },
    );
    // 管理房间是服务器建的，状态事件的发送者也是服务器用户；读房间状态的接口要求有这条。
    state.room_state.insert(
        ADMIN_ROOM_ID.to_owned(),
        rooms::RoomState {
            creator: format!("@{SERVER_USER}:{SERVER}"),
            events: BTreeMap::new(),
        },
    );
}
