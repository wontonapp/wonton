//! 联系人的行为测试：同机的 Bot、对方家里的人、鉴权和重启。外部接口换成内存里的假货，断言联系人列表、
//! “等你确认”列表和对外的副作用（重建 Bot、推送、邀请）。跨机的在 `tests_remote`。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use futures_util::future::BoxFuture;
use wonton_protocol::{
    BotKeeper, BotRef, ContactError, ContactEvent, ContactKind, ContactRequest, ContactRequestPush,
    ContactState, PeerStatus,
};

use super::*;

/// 内存里的几台服务器：发出的事件先排队，测试调 [`Net::deliver_all`] 才送到，送的时候不持有发送方的锁。
#[derive(Default)]
pub(super) struct Net {
    servers: Mutex<BTreeMap<String, Weak<Contacts>>>,
    queue: Mutex<Vec<(String, String, ContactEvent)>>,
    /// 配对状态，没写的已配对服务器按“已连上”。
    reach: Mutex<BTreeMap<String, PeerReach>>,
}

impl Net {
    pub(super) async fn deliver_all(&self) {
        loop {
            let next = {
                let mut queue = self.queue.lock().unwrap();
                (!queue.is_empty()).then(|| queue.remove(0))
            };
            let Some((to, from, event)) = next else {
                return;
            };
            let target = self
                .servers
                .lock()
                .unwrap()
                .get(&to)
                .and_then(Weak::upgrade);
            if let Some(target) = target {
                target.receive(&from, event).await;
            }
        }
    }

    pub(super) fn set_reach(&self, server: &str, reach: PeerReach) {
        self.reach.lock().unwrap().insert(server.to_owned(), reach);
    }

    pub(super) fn queued(&self) -> Vec<ContactEvent> {
        self.queue
            .lock()
            .unwrap()
            .iter()
            .map(|(_, _, e)| e.clone())
            .collect()
    }
}

struct FakeLink {
    own: String,
    net: Arc<Net>,
    fail: Mutex<Option<ContactError>>,
}

impl PeerLink for FakeLink {
    fn reach(&self, server: &str) -> PeerReach {
        if let Some(reach) = self.net.reach.lock().unwrap().get(server) {
            return *reach;
        }
        if self.net.servers.lock().unwrap().contains_key(server) {
            PeerReach::Paired(PeerStatus::Connected)
        } else {
            PeerReach::NotPaired
        }
    }

    fn send(&self, server: &str, event: ContactEvent) -> BoxFuture<'_, Result<(), ContactFailure>> {
        let server = server.to_owned();
        Box::pin(async move {
            if let Some(kind) = *self.fail.lock().unwrap() {
                return Err(ContactFailure::new(kind, "测试让通道失败"));
            }
            self.net
                .queue
                .lock()
                .unwrap()
                .push((server, self.own.clone(), event));
            Ok(())
        })
    }
}

#[derive(Default)]
pub(super) struct FakeDirectory {
    bots: Mutex<Vec<LocalBot>>,
}

impl Directory for FakeDirectory {
    fn bot(&self, bot: &BotRef) -> Result<Option<LocalBot>, String> {
        Ok(self
            .bots
            .lock()
            .unwrap()
            .iter()
            .find(|b| &b.bot == bot)
            .cloned())
    }

    fn bot_by_user(&self, user_id: &str) -> Result<Option<LocalBot>, String> {
        Ok(self
            .bots
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.user_id == user_id)
            .cloned())
    }
}

#[derive(Default)]
pub(super) struct FakeInvites {
    /// 下一次邀请的结果；空时发到，房间 ID 按人编。
    pub(super) next: Mutex<Option<Result<InviteOutcome, ContactFailure>>>,
    pub(super) states: Mutex<BTreeMap<String, InviteeState>>,
    pub(super) sent: Mutex<Vec<String>>,
    pub(super) withdrawn: Mutex<Vec<String>>,
    pub(super) withdraw_fails: Mutex<bool>,
}

impl PersonInvites for FakeInvites {
    fn invite<'a>(
        &'a self,
        _bot: &'a LocalBot,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<InviteOutcome, ContactFailure>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(user_id.to_owned());
            self.next.lock().unwrap().take().unwrap_or_else(|| {
                Ok(InviteOutcome::Sent {
                    room_id: format!("!dm-{user_id}"),
                })
            })
        })
    }

    fn state<'a>(
        &'a self,
        _bot: &'a LocalBot,
        room_id: &'a str,
        _user_id: &'a str,
    ) -> BoxFuture<'a, Result<InviteeState, ContactFailure>> {
        Box::pin(async move {
            Ok(self
                .states
                .lock()
                .unwrap()
                .get(room_id)
                .copied()
                .unwrap_or(InviteeState::Invited))
        })
    }

    fn withdraw<'a>(
        &'a self,
        _bot: &'a LocalBot,
        room_id: &'a str,
        _user_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ContactFailure>> {
        Box::pin(async move {
            if *self.withdraw_fails.lock().unwrap() {
                return Err(ContactFailure::new(ContactError::Upstream, "撤不回"));
            }
            self.withdrawn.lock().unwrap().push(room_id.to_owned());
            Ok(())
        })
    }
}

#[derive(Default)]
pub(super) struct FakeEffects {
    pub(super) rebuilt: Mutex<Vec<BotRef>>,
    pub(super) pushed: Mutex<Vec<(String, ContactRequestPush)>>,
}

impl Effects for FakeEffects {
    fn allowed_users_changed(&self, bot: &BotRef) {
        self.rebuilt.lock().unwrap().push(bot.clone());
    }

    fn notify_request(&self, keeper: &str, push: ContactRequestPush) {
        self.pushed.lock().unwrap().push((keeper.to_owned(), push));
    }
}

/// 一台测试服务器。
pub(super) struct Server {
    pub(super) contacts: Arc<Contacts>,
    pub(super) directory: Arc<FakeDirectory>,
    pub(super) invites: Arc<FakeInvites>,
    pub(super) effects: Arc<FakeEffects>,
    link: Arc<FakeLink>,
    pub(super) dir: tempfile::TempDir,
}

impl Server {
    pub(super) fn start(net: &Arc<Net>, name: &str) -> Self {
        Self::open(net, name, tempfile::tempdir().unwrap(), Arc::default())
    }

    fn open(
        net: &Arc<Net>,
        name: &str,
        dir: tempfile::TempDir,
        directory: Arc<FakeDirectory>,
    ) -> Self {
        let link = Arc::new(FakeLink {
            own: name.to_owned(),
            net: net.clone(),
            fail: Mutex::new(None),
        });
        let invites = Arc::new(FakeInvites::default());
        let effects = Arc::new(FakeEffects::default());
        let contacts = Contacts::open(
            dir.path().join("contacts.json"),
            name,
            Ports {
                directory: directory.clone(),
                link: link.clone(),
                invites: invites.clone(),
                effects: effects.clone(),
            },
        )
        .unwrap();
        net.servers
            .lock()
            .unwrap()
            .insert(name.to_owned(), Arc::downgrade(&contacts));
        Self {
            contacts,
            directory,
            invites,
            effects,
            link,
            dir,
        }
    }

    /// 核心重启：同一份记录重新读。
    pub(super) fn restart(self, net: &Arc<Net>, name: &str) -> Self {
        Self::open(net, name, self.dir, self.directory)
    }

    /// 加一个本机 Bot：私人 Bot 归 `keeper`；`keeper` 是 `owner` 时当共享 Bot。
    pub(super) fn bot(&self, name: &str, keeper: &str) -> BotRef {
        let server = &self.contacts.own_server;
        let bot = BotRef {
            plugin_id: "ai.nous.hermes".into(),
            bot: name.into(),
        };
        let household = keeper == "owner";
        self.directory.bots.lock().unwrap().push(LocalBot {
            bot: bot.clone(),
            user_id: format!("@hermes-{name}:{server}"),
            display_name: Some(format!("Hermes {name}")),
            keeper: if household {
                BotKeeper::Household
            } else {
                BotKeeper::User {
                    username: keeper.into(),
                }
            },
            keeper_username: keeper.into(),
            keeper_display_name: (!household).then(|| keeper.to_uppercase()),
        });
        bot
    }

    pub(super) fn fail_sends(&self, kind: Option<ContactError>) {
        *self.link.fail.lock().unwrap() = kind;
    }

    pub(super) fn row(&self, bot: &BotRef, user: &str) -> Option<BotContact> {
        self.contacts
            .view(bot)
            .into_iter()
            .find(|c| c.user_id == user)
    }

    pub(super) fn rebuilt(&self) -> Vec<String> {
        self.effects
            .rebuilt
            .lock()
            .unwrap()
            .iter()
            .map(|b| b.bot.clone())
            .collect()
    }
}

pub(super) fn bot_request(user: &str) -> ContactRequest {
    ContactRequest {
        user_id: user.into(),
        kind: ContactKind::Bot,
    }
}

pub(super) fn person_request(user: &str) -> ContactRequest {
    ContactRequest {
        user_id: user.into(),
        kind: ContactKind::Person,
    }
}

/// 以管它的人（Bot 名就是归属人的用户名，共享 Bot 归主人）发一条请求。
pub(super) async fn ask(
    server: &Server,
    bot: &BotRef,
    request: ContactRequest,
) -> Result<Vec<BotContact>, ContactFailure> {
    let keeper = server.contacts.local_bot(bot)?.keeper_username;
    server.contacts.request(bot, &keeper, &request).await
}

pub(super) fn state(server: &Server, bot: &BotRef, user: &str) -> Option<ContactState> {
    server.row(bot, user).map(|c| c.state)
}

#[tokio::test]
async fn same_server_bots_become_contacts_once_the_other_keeper_approves() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice_bot = home.bot("alice", "alice");
    let bob_bot = home.bot("bob", "bob");

    let rows = home
        .contacts
        .request(&alice_bot, "alice", &bot_request("@hermes-bob:a.test"))
        .await
        .unwrap();
    assert_eq!(rows[0].state, ContactState::Pending);
    assert_eq!(rows[0].remote, None, "同机的联系人没有服务器那一段");
    assert_eq!(
        state(&home, &bob_bot, "@hermes-alice:a.test"),
        Some(ContactState::Incoming)
    );
    // 等 Bob 处理：进他的“等你确认”，推到他的设备；Alice 那里没有。
    let pending = home.contacts.pending_for("bob");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].bot, bob_bot);
    assert_eq!(
        pending[0].contact.keeper_display_name.as_deref(),
        Some("ALICE")
    );
    assert!(home.contacts.pending_for("alice").is_empty());
    let pushed = home.effects.pushed.lock().unwrap().clone();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].0, "bob");
    assert_eq!(pushed[0].1.user_id, "@hermes-alice:a.test");
    assert!(home.rebuilt().is_empty(), "还没成立，不重建");

    // 不是 Bob 的 Bot，Alice 替他同意不了。
    let refused = home
        .contacts
        .approve(&bob_bot, "alice", "@hermes-alice:a.test")
        .await
        .unwrap_err();
    assert_eq!(refused.kind, ContactError::NotKeeper);

    home.contacts
        .approve(&bob_bot, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    assert_eq!(
        state(&home, &alice_bot, "@hermes-bob:a.test"),
        Some(ContactState::Active)
    );
    assert_eq!(
        state(&home, &bob_bot, "@hermes-alice:a.test"),
        Some(ContactState::Active)
    );
    assert!(home.contacts.pending_for("bob").is_empty());
    let mut rebuilt = home.rebuilt();
    rebuilt.sort();
    assert_eq!(rebuilt, ["alice", "bob"], "两个 Bot 的名单都变了");
    assert_eq!(
        home.contacts.allowed_users("@hermes-alice:a.test"),
        ["@hermes-bob:a.test"]
    );

    // 任何一方删掉，两边都没了，两边都重建。
    home.contacts
        .remove(&bob_bot, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    assert_eq!(state(&home, &alice_bot, "@hermes-bob:a.test"), None);
    assert!(
        home.contacts
            .allowed_users("@hermes-alice:a.test")
            .is_empty()
    );
    assert_eq!(home.rebuilt().len(), 4);
}

#[tokio::test]
async fn a_rejected_request_reads_declined_until_the_requester_clears_it() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice_bot = home.bot("alice", "alice");
    let shared = home.bot("helper", "owner");
    home.contacts
        .request(&alice_bot, "alice", &bot_request("@hermes-helper:a.test"))
        .await
        .unwrap();
    // 共享 Bot 由主人批。
    assert_eq!(home.contacts.pending_for("owner").len(), 1);
    let rows = home
        .contacts
        .reject(&shared, "owner", "@hermes-alice:a.test")
        .await
        .unwrap();
    assert!(rows.is_empty(), "拒绝后这一行消失");
    assert_eq!(
        state(&home, &alice_bot, "@hermes-helper:a.test"),
        Some(ContactState::Declined)
    );
    // 再拒一次：那一行已经不在了。
    let again = home
        .contacts
        .reject(&shared, "owner", "@hermes-alice:a.test")
        .await
        .unwrap_err();
    assert_eq!(again.kind, ContactError::ContactNotFound);
    // 发起方能清掉，也能重新请求。
    home.contacts
        .request(&alice_bot, "alice", &bot_request("@hermes-helper:a.test"))
        .await
        .unwrap();
    assert_eq!(
        state(&home, &alice_bot, "@hermes-helper:a.test"),
        Some(ContactState::Pending)
    );
    assert!(home.rebuilt().is_empty());
}

#[tokio::test]
async fn requests_are_checked_before_anything_is_recorded() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice_bot = home.bot("alice", "alice");
    let request = |user: &str| ask(&home, &alice_bot, bot_request(user));
    for user in [
        "hermes-bob",
        "@hermes-bob",
        "@:a.test",
        "@hermes-alice:a.test",
    ] {
        let refused = request(user).await.unwrap_err();
        assert_eq!(refused.kind, ContactError::InvalidUserId, "{user}");
    }
    let refused = request("@hermes-x:unpaired.test").await.unwrap_err();
    assert_eq!(refused.kind, ContactError::NotPaired);
    let missing = BotRef {
        plugin_id: "ai.nous.hermes".into(),
        bot: "gone".into(),
    };
    let refused = home
        .contacts
        .request(&missing, "alice", &bot_request("@hermes-bob:a.test"))
        .await
        .unwrap_err();
    assert_eq!(refused.kind, ContactError::BotNotFound);
    assert!(home.contacts.view(&alice_bot).is_empty());

    // 本机没有这个 Bot：记成“没有这个人”。再请求一次就是重发，已发出的再请求是重复。
    request("@hermes-nobody:a.test").await.unwrap();
    assert_eq!(
        state(&home, &alice_bot, "@hermes-nobody:a.test"),
        Some(ContactState::NotFound)
    );
    home.bot("bob", "bob");
    request("@hermes-bob:a.test").await.unwrap();
    let refused = request("@hermes-bob:a.test").await.unwrap_err();
    assert_eq!(refused.kind, ContactError::AlreadyContact);
}

#[tokio::test]
async fn requesting_a_bot_that_already_asked_approves_both_sides() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice_bot = home.bot("alice", "alice");
    let bob_bot = home.bot("bob", "bob");
    home.contacts
        .request(&bob_bot, "bob", &bot_request("@hermes-alice:a.test"))
        .await
        .unwrap();
    home.contacts
        .request(&alice_bot, "alice", &bot_request("@hermes-bob:a.test"))
        .await
        .unwrap();
    assert_eq!(
        state(&home, &alice_bot, "@hermes-bob:a.test"),
        Some(ContactState::Active)
    );
    assert_eq!(
        state(&home, &bob_bot, "@hermes-alice:a.test"),
        Some(ContactState::Active)
    );
}

#[tokio::test]
async fn incoming_requests_are_listed_first_and_survive_a_restart() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice_bot = home.bot("alice", "alice");
    home.bot("bob", "bob");
    home.bot("carol", "carol");
    home.contacts
        .request(&alice_bot, "alice", &bot_request("@hermes-bob:a.test"))
        .await
        .unwrap();
    let carol_bot = BotRef {
        plugin_id: "ai.nous.hermes".into(),
        bot: "carol".into(),
    };
    home.contacts
        .request(&carol_bot, "carol", &bot_request("@hermes-alice:a.test"))
        .await
        .unwrap();
    let order: Vec<_> = home
        .contacts
        .view(&alice_bot)
        .into_iter()
        .map(|c| c.state)
        .collect();
    assert_eq!(order, [ContactState::Incoming, ContactState::Pending]);

    let home = home.restart(&net, "a.test");
    let pending = home.contacts.pending_for("alice");
    assert_eq!(pending.len(), 1, "请求不过期，重启后还在");
    assert_eq!(pending[0].contact.user_id, "@hermes-carol:a.test");
    home.contacts
        .approve(&alice_bot, "alice", "@hermes-carol:a.test")
        .await
        .unwrap();
    assert_eq!(
        state(&home, &carol_bot, "@hermes-alice:a.test"),
        Some(ContactState::Active)
    );
}

#[tokio::test]
async fn a_person_becomes_a_contact_only_after_accepting_the_invite() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let _peer = Server::start(&net, "b.test");
    let alice_bot = home.bot("alice", "alice");
    let rows = home
        .contacts
        .request(&alice_bot, "alice", &person_request("@xiaohong:b.test"))
        .await
        .unwrap();
    assert_eq!(rows[0].kind, ContactKind::Person);
    assert_eq!(rows[0].state, ContactState::Pending, "接受前是“等对方接受”");
    assert_eq!(rows[0].remote.as_ref().unwrap().server_name, "b.test");
    assert_eq!(
        rows[0].remote.as_ref().unwrap().peer_status,
        Some(PeerStatus::Connected)
    );
    assert_eq!(*home.invites.sent.lock().unwrap(), ["@xiaohong:b.test"]);
    assert!(net.queued().is_empty(), "人不经核心之间的请求");
    assert!(home.effects.pushed.lock().unwrap().is_empty());

    // 没处理（包括忽略）时一直等着。
    home.contacts.check_people().await;
    assert_eq!(
        state(&home, &alice_bot, "@xiaohong:b.test"),
        Some(ContactState::Pending)
    );
    assert!(
        home.contacts
            .allowed_users("@hermes-alice:a.test")
            .is_empty()
    );

    home.invites
        .states
        .lock()
        .unwrap()
        .insert("!dm-@xiaohong:b.test".into(), InviteeState::Joined);
    let rows = home.contacts.list(&alice_bot, "alice").await.unwrap();
    assert_eq!(rows[0].state, ContactState::Active);
    assert_eq!(home.rebuilt(), ["alice"]);
    assert_eq!(
        home.contacts.allowed_users("@hermes-alice:a.test"),
        ["@xiaohong:b.test"]
    );
}

#[tokio::test]
async fn a_declined_or_unknown_person_is_recorded_and_an_unreachable_one_is_not() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let _peer = Server::start(&net, "b.test");
    let alice_bot = home.bot("alice", "alice");
    let ask = |user: &str| ask(&home, &alice_bot, person_request(user));

    *home.invites.next.lock().unwrap() = Some(Ok(InviteOutcome::Declined));
    ask("@blocked:b.test").await.unwrap();
    assert_eq!(
        state(&home, &alice_bot, "@blocked:b.test"),
        Some(ContactState::Declined)
    );

    *home.invites.next.lock().unwrap() = Some(Ok(InviteOutcome::NotFound));
    ask("@nobody:b.test").await.unwrap();
    assert_eq!(
        state(&home, &alice_bot, "@nobody:b.test"),
        Some(ContactState::NotFound)
    );

    *home.invites.next.lock().unwrap() = Some(Err(ContactFailure::new(
        ContactError::PeerUnreachable,
        "对方服务器 502",
    )));
    let failed = ask("@away:b.test").await.unwrap_err();
    assert_eq!(failed.kind, ContactError::PeerUnreachable);
    assert_eq!(
        state(&home, &alice_bot, "@away:b.test"),
        None,
        "什么都没记下"
    );

    // 探活说连不上时根本不发邀请。
    net.set_reach("b.test", PeerReach::Paired(PeerStatus::Offline));
    let sent_before = home.invites.sent.lock().unwrap().len();
    let failed = ask("@later:b.test").await.unwrap_err();
    assert_eq!(failed.kind, ContactError::PeerUnreachable);
    assert_eq!(home.invites.sent.lock().unwrap().len(), sent_before);

    // 他拒绝了邀请：“对方拒绝了”。
    net.set_reach("b.test", PeerReach::Paired(PeerStatus::Connected));
    ask("@xiaohong:b.test").await.unwrap();
    home.invites
        .states
        .lock()
        .unwrap()
        .insert("!dm-@xiaohong:b.test".into(), InviteeState::Left);
    home.contacts.check_people().await;
    assert_eq!(
        state(&home, &alice_bot, "@xiaohong:b.test"),
        Some(ContactState::Declined)
    );
    assert!(home.rebuilt().is_empty());
}

#[tokio::test]
async fn removing_a_waiting_person_withdraws_the_invite_or_keeps_the_row() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let _peer = Server::start(&net, "b.test");
    let alice_bot = home.bot("alice", "alice");
    home.contacts
        .request(&alice_bot, "alice", &person_request("@xiaohong:b.test"))
        .await
        .unwrap();
    *home.invites.withdraw_fails.lock().unwrap() = true;
    let failed = home
        .contacts
        .remove(&alice_bot, "alice", "@xiaohong:b.test")
        .await
        .unwrap_err();
    assert_eq!(failed.kind, ContactError::Upstream);
    assert_eq!(
        state(&home, &alice_bot, "@xiaohong:b.test"),
        Some(ContactState::Pending)
    );

    *home.invites.withdraw_fails.lock().unwrap() = false;
    home.contacts
        .remove(&alice_bot, "alice", "@xiaohong:b.test")
        .await
        .unwrap();
    assert_eq!(state(&home, &alice_bot, "@xiaohong:b.test"), None);
    assert_eq!(
        *home.invites.withdrawn.lock().unwrap(),
        ["!dm-@xiaohong:b.test"]
    );
}
