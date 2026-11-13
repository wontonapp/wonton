use super::super::{CoreHost, FileHost};
use crate::files::record::Accessor;
use crate::http::HttpClient;
use crate::members::{MemberRecord, Members};
use crate::plugin::bots::{BotEntry, PluginBots};
use crate::plugin::store::Store;
use crate::plugin::{PluginId, PluginManager};
use std::sync::Arc;
use std::time::Duration;
use wonton_protocol::{BotKeeper, MemberStatus};

struct Fixture {
    host: CoreHost,
    members: Arc<Members>,
    store: Store,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let profile = crate::owner::OwnerStore::for_test(
            root,
            &crate::owner::Owner {
                username: "owner".into(),
                display_name: "Owner".into(),
                email: None,
            },
        );
        let store = Store::open(root.join("plugins")).unwrap();
        let config = crate::plugin::lifecycle::LifecycleConfig {
            public: crate::config::PublicUrls {
                scheme: "http".into(),
                base_domain: "test".into(),
                port: 8080,
            },
            data_dir: root.to_path_buf(),
            apps_dir: root.join("apps"),
            shares_dir: root.join("shares"),
            relay_dir: root.join("relay"),
            core_container: None,
            tz: "UTC".into(),
            puid: 1000,
            pgid: 1000,
            install_timeout: Duration::from_secs(5),
            owner: "owner".into(),
            profile,
            gateway_port: 8080,
            plugin_slice: None,
            instance: Default::default(),
        };
        let plugins = Arc::new(PluginManager::new(
            crate::plugin::docker::Engine::unreachable_for_test(),
            Store::open(root.join("plugins")).unwrap(),
            HttpClient::new(),
            None,
            config,
            crate::plugin::image_worker::ImageWorker::in_process(),
        ));
        let records = [
            ("alice", MemberStatus::Joined),
            ("bob", MemberStatus::Joined),
            ("invited", MemberStatus::Invited),
            ("other", MemberStatus::Other),
            ("removed", MemberStatus::Removed),
            ("joined", MemberStatus::Joined),
        ]
        .into_iter()
        .map(|(username, status)| {
            let mut record = MemberRecord::for_test(username);
            record.role_id = "family".into();
            record.status = status;
            record
        })
        .collect::<Vec<_>>();
        std::fs::write(
            root.join("members.json"),
            serde_json::to_vec(&serde_json::json!({"members": records})).unwrap(),
        )
        .unwrap();
        let members = Arc::new(Members::open(root.join("members.json")).unwrap());
        let host = CoreHost::new(
            plugins,
            Arc::new(crate::mcp::upstream::Upstreams::new()),
            None,
        )
        .with_people(
            members.clone(),
            Arc::new(crate::roles::Roles::initial()),
            Arc::new(crate::device::Devices::open(root.join("devices.json")).unwrap()),
            "owner".into(),
            None,
            root.join("core"),
        );
        Self {
            host,
            members,
            store,
            _dir: dir,
        }
    }

    fn bot(&self, plugin: &str, localpart: &str, username: &str) {
        let entry = BotEntry {
            account: crate::matrix::BotAccount {
                user_id: format!("@{localpart}:test"),
                home_room: "!home:test".into(),
            },
            device_id: None,
            keeper: BotKeeper::User {
                username: username.into(),
            },
            bus_token: "test-token".into(),
        };
        let bots = PluginBots {
            bots: [("default".into(), entry)].into(),
        };
        self.store
            .bots()
            .save(&PluginId::parse(plugin).unwrap(), &bots)
            .unwrap();
    }
}

#[tokio::test]
async fn matrix_localparts_disambiguate_same_named_bots_across_plugins() {
    let f = Fixture::new();
    f.bot("app.echo", "echo-user-alice", "alice");
    f.bot("app.other", "echo-user-bob", "bob");
    assert_eq!(
        f.host.bot_person("echo-user-alice").unwrap(),
        Accessor::User("alice".into())
    );
    assert_eq!(
        f.host.bot_person("echo-user-bob").unwrap(),
        Accessor::User("bob".into())
    );
    assert!(f.host.bot_person("default").is_err());
    assert!(f.host.bot_person("missing").is_err());
    f.bot("app.duplicate", "echo-user-alice", "bob");
    assert!(f.host.bot_person("echo-user-alice").is_err());
}

#[tokio::test]
async fn only_joined_members_with_private_home_permission_can_use_files() {
    let f = Fixture::new();
    for username in ["invited", "other", "removed", "joined"] {
        f.bot(
            &format!("app.{username}"),
            &format!("bot-{username}"),
            username,
        );
        let allowed = username == "joined";
        assert_eq!(
            f.host
                .check_person(&Accessor::User(username.into()))
                .is_ok(),
            allowed
        );
        assert_eq!(
            f.host.bot_person(&format!("bot-{username}")).is_ok(),
            allowed
        );
    }
    f.members.set_role("joined", "friend").unwrap();
    assert!(f.host.bot_person("bot-joined").is_err());
    assert!(
        f.host
            .check_person(&Accessor::User("joined".into()))
            .is_err()
    );
}
