//! Role-based app management through real MCP confirmations, with FakeDocker side effects.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::json;
use wonton_protocol::{
    Answer, BotKeeper, CardAudience, CardChoice, ConfirmationCard, ConfirmationState, Gate,
    MemberStatus, Permissions, RoleDraft,
};

use super::agent_install_tests::{INSTALL, bench, plain, plain_root};
use super::test_setup::{Setup, official_package, setup, setup_optional};
use super::tests::failed;
use super::tool_failure_tests::failure_kind;
use super::{AnswerError, Bus, Caller, Confirmations, Draft, Notifier, Refusal};
use crate::members::MemberRecord;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage};

const UNINSTALL: &str = "wonton__uninstall_plugin";
const SET_PERMISSION: &str = "wonton__set_plugin_permission";
const JOURNAL: &str = "app.example.journal";

fn keeper(username: &str) -> Caller {
    Caller::for_bot(
        PluginId::parse("com.example.agent").unwrap(),
        None,
        username.to_owned(),
        BotKeeper::User {
            username: username.to_owned(),
        },
    )
}

fn member(s: &Setup, username: &str, role: &str, status: MemberStatus) {
    let mut record = MemberRecord::for_test(username);
    record.role_id = role.to_owned();
    record.status = status;
    s.members.invite(record).unwrap();
    match status {
        MemberStatus::Joined => {
            s.members.mark_joined(username).unwrap();
        }
        MemberStatus::Removed => {
            s.members.remove(username, false, true).unwrap();
        }
        MemberStatus::Invited => {}
        MemberStatus::Other => panic!("unknown status cannot be created through the member API"),
    }
}

fn role(s: &Setup, gate: Gate, manage_members: bool) -> String {
    s.bus
        .people
        .get()
        .unwrap()
        .roles
        .create(&RoleDraft {
            name: format!("{gate:?}-{manage_members}"),
            permissions: Permissions {
                install_apps: gate,
                manage_members,
                ..Permissions::strictest()
            },
        })
        .unwrap()
        .id
}

async fn journal(s: &Setup) {
    let langs = Languages::from_header(Some("en"));
    let source = official_package(JOURNAL, "Journal", "");
    let package = PluginPackage::parse(&source, &langs).unwrap();
    s.manager
        .install_package(package, &[], langs)
        .await
        .unwrap();
}

async fn card(bus: &Bus, recipient: Option<&str>) -> ConfirmationCard {
    let mut changed = bus.confirmations().subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(card) = bus.confirmations().list(recipient).0.pop() {
                return card;
            }
            changed.changed().await.unwrap();
        }
    })
    .await
    .expect("confirmation was not created")
}

fn has_journal(s: &Setup) -> bool {
    s.manager.is_installed(&PluginId::parse(JOURNAL).unwrap())
}

#[tokio::test]
async fn denied_uninstall_has_no_card_or_mutation_then_allow_asks_and_deletes() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    journal(&s).await;
    member(&s, "alice", "friend", MemberStatus::Joined);
    let alice = keeper("alice");
    let args = json!({"id": JOURNAL, "delete_data": true});
    let denied = s.call_as(&alice, UNINSTALL, args.clone()).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
    assert!(has_journal(&s));
    assert_eq!(s.bus.confirmations().revision(), 0);
    assert!(s.cards.posted.lock().unwrap().is_empty());

    let allow = role(&s, Gate::Allow, false);
    s.members.set_role("alice", &allow).unwrap();
    s.manager
        .tool_policy()
        .set_allow_all(alice.id.as_str(), true)
        .unwrap();
    let call = s.spawn_as(&alice, UNINSTALL, args);
    let pending = card(&s.bus, Some("alice")).await;
    assert_eq!(pending.audience, CardAudience::Keeper);
    assert!(s.bus.confirmations().list(None).0.is_empty());
    assert!(has_journal(&s));
    s.answer(&pending.id, Answer::Approve).unwrap();
    assert!(!failed(&call.await.unwrap()));
    assert!(!has_journal(&s));
    assert_eq!(s.bus.confirmations().list(Some("alice")).1.len(), 1);
}

#[tokio::test]
async fn inactive_missing_and_unknown_keepers_cannot_manage_apps() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    journal(&s).await;
    for (username, status, role_id) in [
        ("invited", MemberStatus::Invited, "admin"),
        ("removed", MemberStatus::Removed, "admin"),
        ("unknown-role", MemberStatus::Joined, "missing"),
    ] {
        member(&s, username, role_id, status);
    }
    let unknown = role(&s, Gate::Other, false);
    member(&s, "unknown-gate", &unknown, MemberStatus::Joined);
    for username in [
        "invited",
        "removed",
        "unknown-role",
        "unknown-gate",
        "missing",
    ] {
        let caller = keeper(username);
        for (tool, args) in [
            (UNINSTALL, json!({"id": JOURNAL})),
            (INSTALL, json!({"id": "app.example.drive"})),
            (INSTALL, json!({"dir": "unread"})),
            (
                SET_PERMISSION,
                json!({"id": JOURNAL, "permission": "docker.socket", "enabled": true}),
            ),
        ] {
            let denied = s.call_as(&caller, tool, args).await;
            assert_eq!(failure_kind(&denied), Some("permission_denied"));
        }
    }
    let unknown = Caller::for_bot(
        s.caller.id.clone(),
        None,
        "unknown".into(),
        BotKeeper::Other,
    );
    let denied = s.call_as(&unknown, UNINSTALL, json!({"id": JOURNAL})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
    assert!(has_journal(&s));
    assert_eq!(s.bus.confirmations().revision(), 0);
    assert!(s.cards.posted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn shared_owner_and_legacy_plugin_callers_preserve_owner_confirmations() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    journal(&s).await;
    let shared = Caller::for_bot(
        s.caller.id.clone(),
        None,
        "shared".into(),
        BotKeeper::Household,
    );
    for caller in [s.caller.clone(), shared, keeper("owner")] {
        let call = s.spawn_as(&caller, UNINSTALL, json!({"id": JOURNAL}));
        let pending = card(&s.bus, None).await;
        assert_eq!(pending.audience, CardAudience::Keeper);
        assert_eq!(
            s.bus
                .confirmations()
                .may_answer(&pending.id, Some("someone")),
            Some(false)
        );
        s.answer(&pending.id, Answer::Reject).unwrap();
        assert_eq!(failure_kind(&call.await.unwrap()), Some("owner_rejected"));
        assert!(has_journal(&s));
    }
}

#[tokio::test]
async fn ask_admin_uninstall_freezes_actual_admins_and_first_answer_wins() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    journal(&s).await;
    let admin = role(&s, Gate::Deny, true);
    let installer = role(&s, Gate::Allow, false);
    member(&s, "alice", "family", MemberStatus::Joined);
    member(&s, "admin", &admin, MemberStatus::Joined);
    member(&s, "installer", &installer, MemberStatus::Joined);
    member(&s, "inactive", &admin, MemberStatus::Invited);
    let call = s.spawn_as(&keeper("alice"), UNINSTALL, json!({"id": JOURNAL}));
    let pending = card(&s.bus, None).await;
    let confirmations = s.bus.confirmations();
    assert_eq!(pending.audience, CardAudience::Admins);
    assert_eq!(confirmations.may_answer(&pending.id, None), Some(true));
    assert_eq!(
        confirmations.may_answer(&pending.id, Some("admin")),
        Some(true)
    );
    for person in ["alice", "installer", "inactive", "unknown"] {
        assert_eq!(
            confirmations.may_answer(&pending.id, Some(person)),
            Some(false)
        );
        assert!(confirmations.list(Some(person)).0.is_empty());
    }
    assert_eq!(confirmations.list(Some("admin")).0[0].id, pending.id);
    assert_eq!(
        confirmations.recipient(&pending.id),
        None,
        "single-recipient API fails closed"
    );
    s.members.set_role("alice", &admin).unwrap();
    s.members.set_role("admin", "friend").unwrap();
    assert_eq!(
        confirmations.may_answer(&pending.id, Some("alice")),
        Some(false)
    );
    assert_eq!(
        confirmations.may_answer(&pending.id, Some("admin")),
        Some(true)
    );
    s.bus
        .answer_card(&pending.id, Answer::Approve, None, "admin")
        .unwrap();
    assert_eq!(
        s.bus
            .answer_card(&pending.id, Answer::Reject, None, "owner"),
        Err(AnswerError::AlreadyAnswered)
    );
    assert!(!failed(&call.await.unwrap()));
    assert!(!has_journal(&s));
    let finished = confirmations.list(Some("admin")).1;
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].id, pending.id);
    assert_eq!(finished[0].answered_by.as_deref(), Some("admin"));
    assert_eq!(confirmations.list(None).1[0].id, pending.id);
    assert!(confirmations.list(Some("alice")).1.is_empty());
}

#[tokio::test]
async fn an_ask_admin_keeper_who_is_already_an_admin_can_answer() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    journal(&s).await;
    let admin = role(&s, Gate::AskAdmin, true);
    member(&s, "alice", &admin, MemberStatus::Joined);
    let call = s.spawn_as(&keeper("alice"), UNINSTALL, json!({"id": JOURNAL}));
    let pending = card(&s.bus, Some("alice")).await;
    assert_eq!(pending.audience, CardAudience::Admins);
    assert_eq!(
        s.bus.confirmations().may_answer(&pending.id, Some("alice")),
        Some(true)
    );
    s.bus
        .answer_card(&pending.id, Answer::Approve, None, "alice")
        .unwrap();
    assert!(!failed(&call.await.unwrap()));
    assert!(!has_journal(&s));
}

#[tokio::test]
async fn ask_admin_rejection_and_expiry_do_not_uninstall() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    journal(&s).await;
    member(&s, "alice", "family", MemberStatus::Joined);
    let alice = keeper("alice");
    let rejected = s.spawn_as(&alice, UNINSTALL, json!({"id": JOURNAL}));
    let pending = card(&s.bus, None).await;
    s.answer(&pending.id, Answer::Reject).unwrap();
    assert_eq!(
        failure_kind(&rejected.await.unwrap()),
        Some("owner_rejected")
    );
    assert!(has_journal(&s));

    let expiring = s.spawn_as(&alice, UNINSTALL, json!({"id": JOURNAL}));
    let pending = card(&s.bus, None).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    let result = expiring.await.unwrap();
    tokio::time::resume();
    assert_eq!(failure_kind(&result), Some("confirmation_expired"));
    assert!(has_journal(&s));
    assert_eq!(
        s.bus.confirmations().card(&pending.id).unwrap().state,
        ConfirmationState::Expired
    );
}

#[tokio::test]
async fn official_install_asks_admins_instead_of_the_keeper() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    member(&s, "alice", "family", MemberStatus::Joined);
    member(&s, "admin", "admin", MemberStatus::Joined);
    let call = s.spawn_as(&keeper("alice"), INSTALL, json!({"id": JOURNAL}));
    let pending = card(&s.bus, Some("admin")).await;
    assert_eq!(pending.audience, CardAudience::Admins);
    assert!(!has_journal(&s));
    assert!(s.bus.confirmations().list(Some("alice")).0.is_empty());
    s.answer(&pending.id, Answer::Approve).unwrap();
    assert!(!failed(&call.await.unwrap()));
    assert!(has_journal(&s));
}

#[tokio::test]
async fn ask_admin_directory_updates_cannot_take_the_quiet_upgrade_path() {
    let b = bench().await;
    let id = "app.agent.hello";
    let (services, manifest) = plain(id, "0.1.0", "");
    b.write_package("hello", services, &manifest);
    let allow = role(&b.s, Gate::Allow, false);
    member(&b.s, "alice", &allow, MemberStatus::Joined);
    let alice = keeper("alice");
    let first = b.s.spawn_as(&alice, INSTALL, json!({"dir": "hello"}));
    let pending = card(&b.s.bus, Some("alice")).await;
    b.s.answer(&pending.id, Answer::Approve).unwrap();
    assert!(!failed(&first.await.unwrap()));
    let id = PluginId::parse(id).unwrap();
    let original = b.s.manager.record_for_test(&id).unwrap();
    b.s.members.set_role("alice", "family").unwrap();
    b.write("hello", "index.html", "<h1>updated</h1>");
    let update = b.s.spawn_as(&alice, INSTALL, json!({"dir": "hello"}));
    let pending = card(&b.s.bus, None).await;
    assert_eq!(pending.audience, CardAudience::Admins);
    assert_eq!(
        b.s.manager.record_for_test(&id).unwrap().files,
        original.files
    );
    b.s.answer(&pending.id, Answer::Reject).unwrap();
    assert_eq!(failure_kind(&update.await.unwrap()), Some("owner_rejected"));
    assert_eq!(
        b.s.manager.record_for_test(&id).unwrap().files,
        original.files
    );
    let update = b.s.spawn_as(&alice, INSTALL, json!({"dir": "hello"}));
    let pending = card(&b.s.bus, None).await;
    b.s.answer(&pending.id, Answer::Approve).unwrap();
    assert!(!failed(&update.await.unwrap()));
    assert_ne!(
        b.s.manager.record_for_test(&id).unwrap().files,
        original.files
    );
}

#[tokio::test]
async fn highest_directory_install_keeps_both_admin_cards() {
    let b = bench().await;
    let id = "app.agent.highest";
    let (services, manifest) = plain_root(id, "0.1.0");
    b.write_package("highest", services, &manifest);
    member(&b.s, "alice", "family", MemberStatus::Joined);
    member(&b.s, "admin", "admin", MemberStatus::Joined);
    let call =
        b.s.spawn_as(&keeper("alice"), INSTALL, json!({"dir": "highest"}));
    let first = card(&b.s.bus, Some("admin")).await;
    assert_eq!(first.audience, CardAudience::Admins);
    b.s.members.set_role("alice", "admin").unwrap();
    b.s.members.set_role("admin", "friend").unwrap();
    b.s.answer(&first.id, Answer::Approve).unwrap();
    let second = card(&b.s.bus, Some("admin")).await;
    assert_ne!(first.id, second.id);
    assert_eq!(second.audience, CardAudience::Admins);
    assert_eq!(
        b.s.bus
            .confirmations()
            .may_answer(&second.id, Some("admin")),
        Some(true)
    );
    assert_eq!(
        b.s.bus
            .confirmations()
            .may_answer(&second.id, Some("alice")),
        Some(false)
    );
    assert!(!b.installed(id));
    b.s.answer(&second.id, Answer::Approve).unwrap();
    assert!(!failed(&call.await.unwrap()));
    assert!(b.installed(id));
    assert_eq!(b.s.bus.confirmations().list(None).1.len(), 2);
}

#[tokio::test]
async fn permission_changes_obey_deny_admin_and_keeper_gates_even_when_relaxed() {
    let s = setup_optional().await;
    let id = PluginId::parse("app.example.notes").unwrap();
    member(&s, "alice", "friend", MemberStatus::Joined);
    let alice = keeper("alice");
    let args =
        |enabled| json!({"id": id.as_str(), "permission": "docker.socket", "enabled": enabled});
    s.manager
        .tool_policy()
        .set_allow_all(alice.id.as_str(), true)
        .unwrap();
    let denied = s.call_as(&alice, SET_PERMISSION, args(true)).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
    assert!(
        s.manager
            .record_for_test(&id)
            .unwrap()
            .enabled_permissions
            .is_empty()
    );
    assert_eq!(s.bus.confirmations().revision(), 0);
    s.members.set_role("alice", "family").unwrap();
    let enable = s.spawn_as(&alice, SET_PERMISSION, args(true));
    let pending = card(&s.bus, None).await;
    assert_eq!(pending.audience, CardAudience::Admins);
    assert_eq!(
        s.bus.confirmations().may_answer(&pending.id, Some("alice")),
        Some(false)
    );
    s.answer(&pending.id, Answer::Approve).unwrap();
    assert!(!failed(&enable.await.unwrap()));
    assert_eq!(
        s.manager
            .record_for_test(&id)
            .unwrap()
            .enabled_permissions
            .len(),
        1
    );
    let allow = role(&s, Gate::Allow, false);
    s.members.set_role("alice", &allow).unwrap();
    let disable = s.spawn_as(&alice, SET_PERMISSION, args(false));
    let pending = card(&s.bus, Some("alice")).await;
    assert_eq!(pending.audience, CardAudience::Keeper);
    assert_eq!(
        s.manager
            .record_for_test(&id)
            .unwrap()
            .enabled_permissions
            .len(),
        1
    );
    s.answer(&pending.id, Answer::Approve).unwrap();
    assert!(!failed(&disable.await.unwrap()));
    assert!(
        s.manager
            .record_for_test(&id)
            .unwrap()
            .enabled_permissions
            .is_empty()
    );
}

#[derive(Default)]
struct AdminDevices {
    recipients: Mutex<Vec<Option<String>>>,
    delivered: tokio::sync::Notify,
}

impl Notifier for AdminDevices {
    fn post<'a>(
        &'a self,
        recipient: Option<&'a str>,
        _card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            self.recipients
                .lock()
                .unwrap()
                .push(recipient.map(str::to_owned));
            self.delivered.notify_one();
            match recipient {
                Some("admin") => Ok(()),
                _ => Err("no paired devices".into()),
            }
        }
        .boxed()
    }
}

fn draft() -> Draft {
    Draft {
        title: "Install".into(),
        lines: Vec::new(),
        choices: [Answer::Approve, Answer::Reject]
            .into_iter()
            .map(|answer| CardChoice {
                answer,
                label: format!("{answer:?}"),
                choice_id: None,
            })
            .collect(),
        scope: None,
        plugin_id: Some(JOURNAL.into()),
        secret_inputs: Vec::new(),
        settings: None,
    }
}

#[tokio::test]
async fn one_unreachable_admin_does_not_remove_the_shared_card() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    member(&s, "admin", "admin", MemberStatus::Joined);
    let notifier = Arc::new(AdminDevices::default());
    let confirmations = Arc::new(Confirmations::new(
        notifier.clone(),
        Duration::from_secs(60),
    ));
    let bus = Bus::new(
        s.manager.clone(),
        confirmations,
        Arc::new(super::audit::Audit::new(
            s.dir.path().join("admin-audit.jsonl"),
        )),
    );
    bus.attach_people(
        "test".into(),
        s.members.clone(),
        s.bus.people.get().unwrap().roles.clone(),
    );
    let ask = tokio::spawn({
        let bus = bus.clone();
        async move { bus.ask_plugin_admin("alice".into(), draft()).await }
    });
    let pending = card(&bus, Some("admin")).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let delivered = notifier.delivered.notified();
            if notifier.recipients.lock().unwrap().len() == 2 {
                break;
            }
            delivered.await;
        }
    })
    .await
    .expect("administrator notifications were not delivered");
    assert_eq!(
        *notifier.recipients.lock().unwrap(),
        [None, Some("admin".into())]
    );
    assert_eq!(bus.confirmations().list(None).0[0].id, pending.id);
    assert_eq!(pending.audience, CardAudience::Admins);
    assert_eq!(
        bus.confirmations().may_answer(&pending.id, Some("alice")),
        Some(false)
    );
    bus.answer_card(&pending.id, Answer::Approve, None, "admin")
        .unwrap();
    let ticket = ask.await.unwrap().unwrap();
    assert_eq!(ticket.id(), pending.id);
    bus.confirmations()
        .finish(
            ticket,
            ConfirmationState::Succeeded {
                message: "done".into(),
            },
        )
        .await;
    assert_eq!(
        bus.confirmations().list(Some("admin")).1[0]
            .answered_by
            .as_deref(),
        Some("admin")
    );

    let bus = Bus::new(
        s.manager.clone(),
        Arc::new(Confirmations::new(notifier, Duration::from_secs(60))),
        Arc::new(super::audit::Audit::new(
            s.dir.path().join("unavailable-audit.jsonl"),
        )),
    );
    assert!(matches!(
        bus.ask_plugin_admin("alice".into(), draft()).await,
        Err(Refusal::Unavailable(_))
    ));
}
