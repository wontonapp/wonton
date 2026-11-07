//! 核心资料工具的可观察行为：自己的 Matrix 资料、outbox 收尾、频率和每次调用的审计。

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use wonton_protocol::{BotActivity, MAX_AVATAR_BYTES};

use super::Bus;
use super::audit::Audit;
use super::test_setup::{NOTES, Setup, manager, setup};
use super::tests::{failed, text};
use crate::files::share_fs::saving::Outgoing;
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{self, Fake, User};
use crate::plugin::docker::Engine;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

const GET: &str = "wonton__get_profile";
const SET: &str = "wonton__set_profile";
const OWN: &str = "@agent:test";
const OTHER: &str = "@other:test";
const PNG: &[u8] = include_bytes!("../../tests/fixtures/avatars/square.png");
const GIF: &[u8] = include_bytes!("../../tests/fixtures/avatars/animated.gif");

struct Bed {
    s: Setup,
    matrix: Fake,
    outbox: PathBuf,
}

impl Bed {
    async fn new() -> Self {
        let mut s = setup(Engine::unreachable_for_test(), Duration::from_secs(5)).await;
        let (fake, cfg) = fake::start().await;
        let matrix = Matrix::connect(&cfg, &s.dir.path().join("matrix"), HttpClient::new())
            .await
            .unwrap();
        let manager = manager(Engine::unreachable_for_test(), &s.dir);
        let manager = Arc::try_unwrap(manager)
            .ok()
            .unwrap()
            .with_matrix(Some(Arc::new(matrix)));
        manager.note_owner_languages(Some("zh"));
        let manager = Arc::new(manager);
        for (id, user) in [(s.caller.id.clone(), OWN), (other_id(), OTHER)] {
            let source = NOTES
                .replace("app.example.notes", id.as_str())
                .replace("permissions: []", "permissions: [matrix.bot]");
            manager.register_for_test(
                PluginPackage::parse(&source, &Languages::from_header(Some("zh-Hans")))
                    .unwrap()
                    .manifest,
                PluginStatus::Running,
                None,
            );
            manager.bot_for_test(&id, user);
            fake.state().users.insert(user.to_owned(), User::default());
        }
        s.bus = Bus::new(
            manager.clone(),
            s.bus.confirmations.clone(),
            Arc::new(Audit::new(s.dir.path().join("audit.jsonl"))),
        );
        s.manager = manager;
        let outbox = s.manager.app_data_dir(&s.caller.id).join("outbox");
        fs::create_dir_all(&outbox).unwrap();
        Self {
            s,
            matrix: fake,
            outbox,
        }
    }

    fn offer(&self, path: &str, data: &[u8]) {
        let path = self.outbox.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    async fn value(&self, tool: &str, args: Value) -> Value {
        let result = self.s.call(tool, args).await;
        assert!(!failed(&result), "{result:?}");
        serde_json::from_str(&text(&result)).unwrap()
    }

    fn no_cards(&self) {
        assert!(self.s.cards.posted.lock().unwrap().is_empty());
    }
}

fn other_id() -> PluginId {
    PluginId::parse("app.example.other").unwrap()
}

#[tokio::test]
async fn tools_read_actual_missing_name_and_rename_only_the_callers_bot_without_confirmation() {
    let b = Bed::new().await;
    let profile = b.value(GET, json!({})).await;
    assert_eq!(profile["name"], Value::Null);
    assert_eq!(profile["avatar"], false);
    assert_eq!(profile["avatars"], json!([]));
    assert_eq!(profile["limits"]["max_bytes"], MAX_AVATAR_BYTES);
    assert_eq!(profile["limits"]["max_edge"], 640);
    assert_eq!(profile["limits"]["max_duration_ms"], 6000);
    let profile = b.value(SET, json!({"name": "  小云  "})).await;
    assert_eq!(profile["name"], "小云");
    assert_eq!(b.value(GET, json!({})).await["name"], "小云");
    assert_eq!(
        b.matrix.state().user(OWN).display_name.as_deref(),
        Some("小云")
    );
    assert_eq!(b.matrix.state().user(OTHER).display_name, None);
    assert_eq!(
        b.s.manager.profile_revisions().get(b.s.caller.id.as_str()),
        1
    );
    b.no_cards();
    let audit = b.s.audit();
    assert_eq!(audit.len(), 3);
    assert!(
        audit
            .iter()
            .all(|row| row["decision"] == "allowed" && row["outcome"] == "ok")
    );
    assert!(
        audit
            .iter()
            .all(|row| row["caller"] == b.s.caller.id.as_str())
    );
}

#[tokio::test]
async fn unknown_targeting_arguments_and_invalid_mutations_are_rejected_and_audited() {
    let b = Bed::new().await;
    let mut calls = 0;
    for field in ["id", "plugin_id", "user_id", "target", "bot"] {
        for tool in [GET, SET] {
            let mut args = json!({"name": "不能改"});
            args[field] = json!(OTHER);
            let result = b.s.call(tool, args).await;
            assert!(failed(&result), "{field}: {result:?}");
            calls += 1;
        }
    }
    for args in [
        json!({}),
        json!({"avatars": {}}),
        json!({"avatar": null}),
        json!({"name": null}),
        json!({"name": 3}),
        json!({"avatars": {"other": null}}),
        json!({"avatars": {"idle": 7}}),
    ] {
        assert!(failed(&b.s.call(SET, args).await));
        calls += 1;
    }
    let state = b.matrix.state();
    assert_eq!(state.user(OWN).display_name, None);
    assert_eq!(state.user(OTHER).display_name, None);
    drop(state);
    let audit = b.s.audit();
    assert_eq!(audit.len(), calls);
    assert!(audit.iter().all(|row| row["decision"] == "denied"));
    assert_eq!(
        b.s.manager.profile_revisions().get(b.s.caller.id.as_str()),
        0
    );
    b.no_cards();
}

#[tokio::test]
async fn no_bot_and_uninstalled_callers_cannot_read_or_write_profiles() {
    let b = Bed::new().await;
    for id in ["app.example.nobot", "app.example.uninstalled"] {
        let id = PluginId::parse(id).unwrap();
        if id.as_str().ends_with("nobot") {
            let source = NOTES.replace("app.example.notes", id.as_str());
            b.s.manager.register_for_test(
                PluginPackage::parse(&source, &Languages::from_header(Some("zh-Hans")))
                    .unwrap()
                    .manifest,
                PluginStatus::Running,
                None,
            );
            b.s.manager.bot_for_test(&id, OWN);
        } else {
            b.s.manager.bot_for_test(&id, OWN);
        }
        let caller = super::Caller::new(id, None);
        for (tool, args) in [(GET, json!({})), (SET, json!({"name": "别动"}))] {
            let request = rmcp::model::CallToolRequestParams::new(tool)
                .with_arguments(args.as_object().unwrap().clone());
            let result = b.s.bus.call(&caller, request, std::future::pending()).await;
            assert!(failed(&result));
        }
    }
    assert_eq!(b.matrix.state().user(OWN).display_name, None);
    assert_eq!(b.s.audit().len(), 4);
    b.no_cards();
}

#[tokio::test]
async fn successful_avatar_writes_consume_only_used_files_and_preserve_other_states() {
    let b = Bed::new().await;
    b.offer("portrait.bin", PNG);
    b.offer("same.bin", GIF);
    b.offer("unused.png", PNG);
    let profile = b
        .value(
            SET,
            json!({"avatar": "portrait.bin", "avatars": {
                "idle": "same.bin", "working": "same.bin", "waiting": "same.bin"
            }}),
        )
        .await;
    assert_eq!(profile["avatar"], true);
    assert_eq!(profile["avatars"], json!(["idle", "working", "waiting"]));
    assert!(!b.outbox.join("portrait.bin").exists());
    assert!(!b.outbox.join("same.bin").exists());
    assert!(b.outbox.join("unused.png").exists());
    let before = b.s.manager.bot_profile(&b.s.caller.id).await.unwrap();
    assert_eq!(before.avatars.states["working"].mimetype, "image/gif");
    let profile = b.value(SET, json!({"avatars": {"idle": null}})).await;
    assert_eq!(profile["avatars"], json!(["working", "waiting"]));
    let after = b.s.manager.bot_profile(&b.s.caller.id).await.unwrap();
    assert_eq!(
        after.avatars.states["working"],
        before.avatars.states["working"]
    );
    assert_eq!(
        after.avatars.states["waiting"],
        before.avatars.states["waiting"]
    );
    assert!(b.matrix.state().user(OWN).avatar_url.is_some());
    assert!(b.matrix.state().user(OTHER).avatar_url.is_none());
    assert_eq!(
        b.s.manager.profile_revisions().get(b.s.caller.id.as_str()),
        2
    );
    b.no_cards();
}

#[tokio::test]
async fn rejected_media_names_and_matrix_failures_never_consume_outbox_files() {
    let b = Bed::new().await;
    b.offer("valid.png", PNG);
    b.offer("animated.gif", GIF);
    b.offer("invalid.png", b"not an image");
    b.offer("oversize.png", &vec![0; MAX_AVATAR_BYTES as usize + 2]);
    let args = [
        json!({"name": "Wonton", "avatar": "valid.png"}),
        json!({"avatar": "animated.gif"}),
        json!({"avatar": "invalid.png"}),
        json!({"avatar": "oversize.png"}),
    ];
    for args in args {
        assert!(failed(&b.s.call(SET, args).await));
    }
    b.matrix.state().refuse_profile = true;
    assert!(failed(&b.s.call(SET, json!({"avatar": "valid.png"})).await));
    assert!(failed(&b.s.call(GET, json!({})).await));
    b.matrix.state().refuse_profile = false;
    for file in ["valid.png", "animated.gif", "invalid.png", "oversize.png"] {
        assert!(b.outbox.join(file).exists());
    }
    assert_eq!(b.matrix.state().user(OWN).avatar_url, None);
    assert_eq!(
        b.s.manager.profile_revisions().get(b.s.caller.id.as_str()),
        0
    );
    let audit = b.s.audit();
    assert_eq!(audit.len(), 6);
    assert!(audit.iter().all(|row| row["decision"] == "denied"));
    assert_eq!(b.value(SET, json!({"name": "恢复"})).await["name"], "恢复");
    b.no_cards();
}

#[tokio::test]
async fn traversal_symlinks_and_other_plugins_outboxes_are_never_read() {
    let b = Bed::new().await;
    b.offer("valid.png", PNG);
    let other = b.s.manager.app_data_dir(&other_id()).join("outbox");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("secret.png"), PNG).unwrap();
    std::os::unix::fs::symlink(&other, b.outbox.join("linked")).unwrap();
    std::os::unix::fs::symlink(other.join("secret.png"), b.outbox.join("link.png")).unwrap();
    let absolute = other.join("secret.png").to_str().unwrap().to_owned();
    for path in [
        "../secret.png",
        "linked/secret.png",
        "link.png",
        "a/../valid.png",
        "mxc://test/id",
        &absolute,
    ] {
        assert!(failed(&b.s.call(SET, json!({"avatar": path})).await));
    }
    assert!(other.join("secret.png").exists());
    assert!(b.outbox.join("valid.png").exists());
    assert!(b.matrix.state().media.is_empty());
    assert_eq!(b.s.audit().len(), 6);
    b.no_cards();
}

#[tokio::test]
async fn only_successes_count_towards_the_ten_per_hour_limit_and_rejections_preserve_files() {
    let b = Bed::new().await;
    for _ in 0..12 {
        assert!(failed(&b.s.call(SET, json!({"name": "Wonton"})).await));
    }
    for i in 0..10 {
        b.value(SET, json!({"name": format!("小云{i}")})).await;
    }
    b.offer("limited.png", PNG);
    let result = b.s.call(SET, json!({"avatar": "limited.png"})).await;
    assert!(failed(&result));
    let message = text(&result);
    assert!(message.starts_with("rate_limited:"));
    let retry = message
        .split("retry_after_secs=")
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!((1..=3600).contains(&retry));
    assert!(b.outbox.join("limited.png").exists());
    assert_eq!(
        b.s.manager.profile_revisions().get(b.s.caller.id.as_str()),
        10
    );
    assert_eq!(b.value(GET, json!({})).await["name"], "小云9");
    let audit = b.s.audit();
    assert_eq!(audit.len(), 24);
    assert_eq!(audit[22]["decision"], "denied");
    assert_eq!(audit[23]["outcome"], "ok");
    b.no_cards();
}

#[tokio::test]
async fn concurrent_mutations_do_not_exceed_the_limit() {
    let b = Bed::new().await;
    let calls = (0..12).map(|i| b.s.spawn(SET, json!({"name": format!("并发{i}")})));
    let results = futures_util::future::join_all(calls).await;
    let succeeded = results
        .into_iter()
        .map(Result::unwrap)
        .filter(|result| !failed(result))
        .count();
    assert_eq!(succeeded, 10);
    assert_eq!(
        b.s.manager.profile_revisions().get(b.s.caller.id.as_str()),
        10
    );
    assert_eq!(b.s.audit().len(), 12);
    b.no_cards();
}

#[test]
fn bounded_outbox_reads_keep_rewritten_and_replaced_files_when_consuming_originals() {
    let dir = tempfile::tempdir().unwrap();
    let outbox = dir.path().join("outbox");
    fs::create_dir(&outbox).unwrap();
    for name in ["unchanged", "rewritten", "replaced"] {
        fs::write(outbox.join(name), b"original").unwrap();
    }
    let paths = ["unchanged", "rewritten", "replaced"].map(str::to_owned);
    let outgoing = Outgoing::open(dir.path(), &paths).unwrap();
    let data = outgoing.read_bounded(4).unwrap();
    assert!(data.iter().all(|(_, data)| data == b"orig"));
    fs::write(outbox.join("rewritten"), b"new contents").unwrap();
    fs::remove_file(outbox.join("replaced")).unwrap();
    fs::write(outbox.join("replaced"), b"replacement").unwrap();
    outgoing.remove_originals();
    assert!(!outbox.join("unchanged").exists());
    assert_eq!(fs::read(outbox.join("rewritten")).unwrap(), b"new contents");
    assert_eq!(fs::read(outbox.join("replaced")).unwrap(), b"replacement");
}

#[test]
fn changed_during_reading_is_rejected_and_symlinked_outbox_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let outbox = dir.path().join("outbox");
    fs::create_dir(&outbox).unwrap();
    fs::write(outbox.join("avatar"), PNG).unwrap();
    let outgoing = Outgoing::open(dir.path(), &["avatar".to_owned()]).unwrap();
    fs::write(outbox.join("avatar"), b"changed").unwrap();
    assert!(outgoing.read_bounded(MAX_AVATAR_BYTES + 1).is_err());
    assert!(outbox.join("avatar").exists());
    fs::rename(&outbox, dir.path().join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("elsewhere"), &outbox).unwrap();
    assert!(Outgoing::open(dir.path(), &["avatar".to_owned()]).is_err());
}

#[tokio::test]
async fn only_explicitly_populated_known_states_are_reported() {
    let b = Bed::new().await;
    b.offer("idle.png", PNG);
    b.value(SET, json!({"avatars": {"idle": "idle.png"}})).await;
    let profile = b.s.manager.bot_profile(&b.s.caller.id).await.unwrap();
    assert!(profile.avatars.for_state(BotActivity::Offline).is_some());
    assert_eq!(b.value(GET, json!({})).await["avatars"], json!(["idle"]));
}

#[test]
fn outbox_media_reads_stop_one_byte_after_the_fixed_protocol_limit() {
    let dir = tempfile::tempdir().unwrap();
    let outbox = dir.path().join("outbox");
    fs::create_dir(&outbox).unwrap();
    fs::write(
        outbox.join("large"),
        vec![42; MAX_AVATAR_BYTES as usize + 100],
    )
    .unwrap();
    let outgoing = Outgoing::open(dir.path(), &["large".to_owned()]).unwrap();
    let data = outgoing.read_bounded(MAX_AVATAR_BYTES + 1).unwrap();
    assert_eq!(data[0].1.len() as u64, MAX_AVATAR_BYTES + 1);
    assert!(outbox.join("large").exists());
}
