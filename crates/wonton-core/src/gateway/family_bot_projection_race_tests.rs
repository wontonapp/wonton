use std::sync::{Arc, mpsc};
use std::time::Duration;

use hyper::{Method, StatusCode};
use iroh::{EndpointId, SecretKey};
use serde_json::{Value, json};
use wonton_protocol::{BotKeeper, BotRef};

use crate::contacts::{Contacts, Directory, LocalBot, Ports};
use crate::gateway::bot_contacts_tests::{Offline, Recorded};
use crate::gateway::bot_projection::bot_projection_test_gate;
use crate::gateway::family_tests::{PORT, send_as};
use crate::gateway::tests::{CORE, Harness, Reply, TOKEN, WHOAMI, owner};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

struct OneBot(LocalBot);

impl Directory for OneBot {
    fn bot(&self, bot: &BotRef) -> Result<Option<LocalBot>, String> {
        Ok((&self.0.bot == bot).then(|| self.0.clone()))
    }

    fn bot_by_user(&self, user_id: &str) -> Result<Option<LocalBot>, String> {
        Ok((self.0.user_id == user_id).then(|| self.0.clone()))
    }
}

struct Rig {
    home: Harness,
    app: axum::Router,
    contacts: Arc<Contacts>,
    bot: BotRef,
    plugin: PluginId,
    device: EndpointId,
    _matrix_state: tempfile::TempDir,
}

impl Rig {
    async fn new(keeper: BotKeeper, active_contact: bool) -> Self {
        let home = Harness::start(None).await;
        let plugin = PluginId::parse("app.whoami").unwrap();
        let bot = BotRef {
            plugin_id: plugin.to_string(),
            bot: "default".into(),
        };
        let mut contacts = json!({});
        if active_contact {
            contacts["@mia:test"] =
                json!({"kind": "person", "state": "active", "updated_at_ms": 1});
        }
        std::fs::write(
            home.data_dir.join("contact-facts.json"),
            serde_json::to_vec(&json!({"bots": {"app.whoami/default": {
                "bot": &bot, "contacts": contacts
            }}}))
            .unwrap(),
        )
        .unwrap();
        let source = WHOAMI
            .replace("  kind: app", "  kind: agent\n  agent: {protocol: matrix}")
            .replace(
                "  permissions: []",
                "  bot: {display_name: {zh: 助手, en: Assistant}}\n  bots: {services: [web]}\n  permissions: [matrix.bot]",
            );
        home.manager.register_for_test(
            PluginPackage::manifest_of(&source, &Languages::default()).unwrap(),
            PluginStatus::Running,
            None,
        );
        home.manager.bot_records_for_test(
            &plugin,
            &[("default", "@private:test", keeper.clone(), "test-token")],
        );
        home.members
            .invite(MemberRecord {
                invited_by: None,
                role_id: "admin".into(),
                ..MemberRecord::for_test("mia")
            })
            .unwrap();
        home.members.mark_joined("mia").unwrap();
        let device = SecretKey::generate().public();
        home.tunnel
            .devices
            .add_if(device, None, Some("mia".into()), |_| true)
            .unwrap();
        let (app, contacts, matrix_state) = app(&home, &bot, keeper).await;
        Self {
            home,
            app,
            contacts,
            bot,
            plugin,
            device,
            _matrix_state: matrix_state,
        }
    }

    async fn pause_detail(&self) -> Pending {
        let app = self.app.clone();
        let device = self.device;
        let (reached, at_read) = tokio::sync::oneshot::channel();
        let (release, released) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            bot_projection_test_gate::install(move || {
                reached.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(30)).unwrap();
            });
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(detail(&app, device))
        });
        tokio::time::timeout(Duration::from_secs(30), at_read)
            .await
            .expect("the first fact read did not reach the gate")
            .expect("the request ended before the first fact read");
        Pending { release, thread }
    }
}

async fn app(
    home: &Harness,
    bot: &BotRef,
    keeper: BotKeeper,
) -> (axum::Router, Arc<Contacts>, tempfile::TempDir) {
    let (_, config) = crate::matrix::fake::start().await;
    let matrix_state = tempfile::tempdir().unwrap();
    let matrix = Arc::new(
        crate::matrix::Matrix::connect(&config, matrix_state.path(), home.http.clone())
            .await
            .unwrap(),
    );
    let owner = crate::owner::OwnerStore::for_test(&home.data_dir, &owner());
    let keeper_username = match &keeper {
        BotKeeper::User { username } => username.clone(),
        BotKeeper::Household => "owner".into(),
        BotKeeper::Other => panic!("unknown keeper is not used by this fixture"),
    };
    let directory = OneBot(LocalBot {
        bot: bot.clone(),
        user_id: "@private:test".into(),
        display_name: None,
        keeper,
        keeper_username,
        keeper_display_name: None,
    });
    let contacts = Contacts::open(
        home.data_dir.join("contact-facts.json"),
        matrix.server_name(),
        Ports {
            directory: Arc::new(directory),
            link: Arc::new(Offline),
            invites: Arc::new(Offline),
            effects: Arc::new(Recorded::default()),
            open_federation: false,
        },
    )
    .unwrap();
    let gateway = crate::gateway::Gateway::new(
        crate::config::PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: PORT,
        },
        owner,
        TOKEN.into(),
        home.members.clone(),
        home.manager.clone(),
        None,
        home.http.clone(),
        home.tunnel.clone(),
    )
    .with_roles(home.roles.clone())
    .with_matrix(Some(matrix))
    .with_contacts(Some(contacts.clone()));
    (
        crate::gateway::router(Arc::new(gateway)),
        contacts,
        matrix_state,
    )
}

struct Pending {
    release: mpsc::Sender<()>,
    thread: std::thread::JoinHandle<Reply>,
}

impl Pending {
    async fn finish(self) -> Reply {
        self.release.send(()).unwrap();
        tokio::task::spawn_blocking(move || self.thread.join().unwrap())
            .await
            .unwrap()
    }
}

async fn detail(app: &axum::Router, device: EndpointId) -> Reply {
    send_as(
        app,
        Some(device),
        Method::GET,
        CORE,
        "/_wonton/api/plugins/app.whoami",
        &[],
        "",
    )
    .await
}

fn owner_bot() -> BotKeeper {
    BotKeeper::User {
        username: "owner".into(),
    }
}

fn assert_unknown(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(
        reply.json()["installed"]["default_bot_visibility"],
        "unknown"
    );
    assert_eq!(reply.json()["installed"]["bot_user_id"], Value::Null);
    assert!(!reply.text().contains("@private:test"));
}

fn assert_visible(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(
        reply.json()["installed"]["default_bot_visibility"],
        "visible"
    );
    assert_eq!(reply.json()["installed"]["bot_user_id"], "@private:test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_identity_rechecks_facts_before_disclosure() {
    let contact = Rig::new(owner_bot(), true).await;
    assert_visible(&detail(&contact.app, contact.device).await);
    let pending = contact.pause_detail().await;
    contact
        .contacts
        .remove(&contact.bot, "owner", "@mia:test")
        .await
        .unwrap();
    assert_unknown(&pending.finish().await);

    let replaced = Rig::new(owner_bot(), true).await;
    let pending = replaced.pause_detail().await;
    replaced.home.manager.bot_records_for_test(
        &replaced.plugin,
        &[("default", "@replacement:test", owner_bot(), "test-token")],
    );
    let reply = pending.finish().await;
    assert_unknown(&reply);
    assert!(!reply.text().contains("@replacement:test"));

    let keeper_changed = Rig::new(owner_bot(), true).await;
    let pending = keeper_changed.pause_detail().await;
    keeper_changed.home.manager.bot_records_for_test(
        &keeper_changed.plugin,
        &[(
            "default",
            "@private:test",
            BotKeeper::User {
                username: "mia".into(),
            },
            "test-token",
        )],
    );
    assert_unknown(&pending.finish().await);

    let corrupt = Rig::new(owner_bot(), true).await;
    let pending = corrupt.pause_detail().await;
    std::fs::write(
        corrupt.home.data_dir.join("plugins/bots/app.whoami.json"),
        "{",
    )
    .unwrap();
    let reply = pending.finish().await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_kind(), Some("bot_record_corrupt"));
    assert!(!reply.text().contains("@private:test"));

    for keeper in [
        BotKeeper::User {
            username: "mia".into(),
        },
        BotKeeper::Household,
    ] {
        let independent = Rig::new(keeper, false).await;
        let reply = detail(&independent.app, independent.device).await;
        assert_visible(&reply);
    }
}

#[tokio::test]
async fn malformed_bot_identity_is_not_disclosed() {
    let rig = Rig::new(
        BotKeeper::User {
            username: "mia".into(),
        },
        false,
    )
    .await;
    rig.home.manager.bot_records_for_test(
        &rig.plugin,
        &[(
            "default",
            "not-a-matrix-id",
            BotKeeper::User {
                username: "mia".into(),
            },
            "test-token",
        )],
    );
    let reply = detail(&rig.app, rig.device).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_kind(), Some("invalid_bot_record"));
    assert!(!reply.text().contains("not-a-matrix-id"));
}

#[tokio::test]
async fn bot_identity_from_another_homeserver_is_not_disclosed() {
    let rig = Rig::new(
        BotKeeper::User {
            username: "mia".into(),
        },
        false,
    )
    .await;
    rig.home.manager.bot_records_for_test(
        &rig.plugin,
        &[(
            "default",
            "@private:elsewhere",
            BotKeeper::User {
                username: "mia".into(),
            },
            "test-token",
        )],
    );
    let reply = detail(&rig.app, rig.device).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_kind(), Some("invalid_bot_record"));
    assert!(!reply.text().contains("@private:elsewhere"));
}
