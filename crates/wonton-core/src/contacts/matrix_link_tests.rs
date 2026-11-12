//! 联系人通道对假 Tuwunel 的行为：邀请加入失败、配对晚到、建房失败时怎么办。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::sync::watch;
use wonton_protocol::{CONTACT_ROOM_TYPE, ContactError, ContactEvent, PeerStatus};

use super::{MatrixLink, PairedServers, PeerLink};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{Fake, Room, start};

/// 内存里的配对表。
struct Paired {
    names: watch::Sender<BTreeSet<String>>,
    reach: Mutex<BTreeMap<String, PeerStatus>>,
}

impl Paired {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            names: watch::channel(BTreeSet::new()).0,
            reach: Mutex::default(),
        })
    }

    fn pair(&self, server: &str) {
        self.reach
            .lock()
            .unwrap()
            .insert(server.to_owned(), PeerStatus::Connected);
        self.names.send_modify(|names| {
            names.insert(server.to_owned());
        });
    }
}

impl PairedServers for Paired {
    fn paired_names(&self) -> watch::Receiver<BTreeSet<String>> {
        self.names.subscribe()
    }

    fn contact_reach(&self, server_name: &str) -> Option<PeerStatus> {
        self.reach.lock().unwrap().get(server_name).copied()
    }
}

struct Side {
    link: Arc<MatrixLink>,
    fake: Fake,
    paired: Arc<Paired>,
    me: String,
    _dir: tempfile::TempDir,
}

/// 本机是假 Tuwunel 的 `test`。
async fn side() -> Side {
    let dir = tempfile::tempdir().unwrap();
    let (fake, config) = start().await;
    let matrix = Arc::new(
        Matrix::connect(&config, dir.path(), HttpClient::new())
            .await
            .unwrap(),
    );
    let paired = Paired::new();
    let link = MatrixLink::open(
        dir.path().join("contact-rooms.json"),
        matrix.clone(),
        paired.clone(),
    )
    .unwrap();
    Side {
        link,
        fake,
        paired,
        me: matrix.admin_user_id(),
        _dir: dir,
    }
}

/// `@wonton-core:<server>` 邀请本机核心账号进联系人房间时 `/sync` 里的样子。
fn invite_from(server: &str, me: &str) -> serde_json::Value {
    json!({"invite_state": {"events": [
        {"type": "m.room.create", "state_key": "", "content": {"type": CONTACT_ROOM_TYPE}},
        {"type": "m.room.member", "state_key": me, "sender": format!("@wonton-core:{server}"),
         "content": {"membership": "invite"}},
    ]}})
}

/// 对方建好的联系人房间，本机核心账号在邀请里。
fn peer_room(side: &Side, room: &str, server: &str) {
    side.fake.state().rooms.insert(
        room.to_owned(),
        Room {
            members: BTreeSet::from([format!("@wonton-core:{server}")]),
            invited: BTreeSet::from([side.me.clone()]),
            ..Room::default()
        },
    );
}

fn joined(side: &Side, room: &str) -> bool {
    side.fake.state().rooms[room].members.contains(&side.me)
}

fn remove_event() -> ContactEvent {
    ContactEvent::Remove {
        from: "@hermes-a:test".into(),
        to: "@hermes-b:zeta".into(),
    }
}

#[tokio::test]
async fn an_invite_that_could_not_be_joined_is_joined_on_a_later_try() {
    let side = side().await;
    side.paired.pair("alpha");
    // 邀请到了，但加入失败（对方的房间这时从这边还够不着）；之后的增量同步不会再带上这条邀请。
    side.link
        .consider_invite("!c1:alpha", &invite_from("alpha", &side.me));
    assert!(!side.link.join_invited().await);

    peer_room(&side, "!c1:alpha", "alpha");
    assert!(side.link.join_invited().await);
    assert!(joined(&side, "!c1:alpha"));
    assert_eq!(side.link.lock().rooms["alpha"].room_id, "!c1:alpha");
    assert!(side.link.lock().joins.is_empty(), "加入了就不再试");
}

#[tokio::test]
async fn an_invite_arriving_before_the_pairing_is_recorded_is_joined_once_paired() {
    let side = side().await;
    peer_room(&side, "!c1:alpha", "alpha");
    side.link
        .consider_invite("!c1:alpha", &invite_from("alpha", &side.me));
    assert!(side.link.join_invited().await, "没配对的不算没办成");
    assert!(!joined(&side, "!c1:alpha"));

    side.paired.pair("alpha");
    assert!(side.link.join_invited().await);
    assert!(joined(&side, "!c1:alpha"));
}

#[tokio::test]
async fn an_invite_the_peer_should_not_have_sent_is_not_recorded() {
    let side = side().await;
    side.paired.pair("zeta");
    // 名字更大的一方不建房：它发来的邀请不接受。
    side.link
        .consider_invite("!c1:zeta", &invite_from("zeta", &side.me));
    assert!(side.link.lock().joins.is_empty());
}

#[tokio::test]
async fn sending_before_the_room_exists_tells_a_local_failure_from_an_unreachable_peer() {
    let side = side().await;
    side.paired.pair("zeta");
    side.fake.state().refuse_rooms = true;
    assert!(!side.link.ensure_rooms().await);
    let failed = side.link.send("zeta", remove_event()).await.unwrap_err();
    assert_eq!(failed.kind, ContactError::Upstream, "建房失败是本机的事");

    side.fake.state().refuse_rooms = false;
    assert!(side.link.ensure_rooms().await);
    let failed = side.link.send("zeta", remove_event()).await.unwrap_err();
    assert_eq!(
        failed.kind,
        ContactError::PeerUnreachable,
        "房间好了、对方还没进来"
    );
}
