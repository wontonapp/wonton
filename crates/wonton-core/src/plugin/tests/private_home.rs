use std::sync::Arc;

use serde_json::Value;
use wonton_protocol::{AiLevel, BotKeeper, MemberStatus, RoleDraft};

use super::super::fake_docker::FakeDocker;
use super::super::manager::PluginManager;
use super::super::manifest::{Permission, PluginId};
use super::super::package::PluginPackage;
use super::super::render::{dry_run_context, render};
use super::manager::manager_in_container;
use crate::members::{MemberRecord, Members};
use crate::roles::Roles;

pub(super) fn compose(image: &str) -> String {
    super::package(
        &format!("  bot:\n    image: {image}\n    command: ['sleep', 'infinity']\n    volumes: ['${{WONTON_BOT_DATA}}:/data']\n    environment:\n      TOKEN: ${{WONTON_MATRIX_ACCESS_TOKEN}}\n      USER: ${{WONTON_MATRIX_USER_ID}}\n  web:\n    image: {image}\n    command: ['sleep', 'infinity']"),
        "  agent: {protocol: matrix}\n  bots: {services: [bot]}\n  permissions: [matrix.bot, storage.home:write]",
    ).replace("kind: app", "kind: agent")
}

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}
fn permissions() -> Vec<Permission> {
    vec![Permission::MatrixBot, "storage.home:write".parse().unwrap()]
}
fn container(bot: &str) -> String {
    super::super::names::Instance::default().bot_container("app-test", "bot", bot)
}
fn mount(docker: &FakeDocker, bot: &str) -> Value {
    docker.created(&container(bot)).unwrap()["HostConfig"]["Mounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mount| mount["Target"] == "/wonton/home")
        .unwrap()
        .clone()
}

struct Bed {
    dir: tempfile::TempDir,
    docker: FakeDocker,
    matrix: crate::matrix::fake::Fake,
    manager: PluginManager,
    roles: Arc<Roles>,
}
impl Bed {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (docker, engine) = FakeDocker::start().await;
        let (matrix, cfg) = crate::matrix::fake::start().await;
        let client = crate::matrix::Matrix::connect(
            &cfg,
            &dir.path().join("core"),
            crate::http::HttpClient::new(),
        )
        .await
        .unwrap();
        let manager = manager_in_container(engine, &dir).with_matrix(Some(Arc::new(client)));
        let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
        let roles = Arc::new(Roles::initial());
        let mut role = roles.get("family").unwrap().permissions;
        role.ai = AiLevel::SelfConfirms;
        roles
            .update(
                "family",
                &RoleDraft {
                    name: "Family".into(),
                    permissions: role,
                },
            )
            .unwrap();
        for name in ["alice", "bob"] {
            let mut member = MemberRecord::for_test(name);
            member.status = MemberStatus::Joined;
            member.role_id = "family".into();
            members.invite(member).unwrap();
            members.mark_joined(name).unwrap();
        }
        manager.attach_person_state(members, roles.clone());
        manager
            .install(&compose("echo"), &permissions(), super::langs())
            .await
            .unwrap();
        Self {
            dir,
            docker,
            matrix,
            manager,
            roles,
        }
    }
}

#[tokio::test]
async fn owners_have_isolated_mounts_and_role_downgrades_rebuild_only_changed_bots() {
    let bed = Bed::start().await;
    let alice = bed
        .manager
        .ensure_member_bot(&id(), "alice", "alice")
        .await
        .unwrap();
    bed.manager
        .ensure_member_bot(&id(), "bob", "bob")
        .await
        .unwrap();
    for (bot, owner) in [("default", "owner"), ("alice", "alice"), ("bob", "bob")] {
        assert_eq!(
            mount(&bed.docker, bot)["Source"],
            bed.dir
                .path()
                .join("core/home")
                .join(owner)
                .display()
                .to_string()
        );
        assert_eq!(mount(&bed.docker, bot)["ReadOnly"], false);
    }
    let shared = bed
        .docker
        .created(&super::super::names::container_name("app-test", "web"))
        .unwrap();
    assert!(
        !shared["HostConfig"]["Mounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|mount| mount["Target"] == "/wonton/home")
    );
    let original = bed
        .manager
        .store
        .bots()
        .entry(&id(), "alice")
        .unwrap()
        .unwrap();
    let count = bed.docker.creations(&container("alice"));
    bed.manager
        .ensure_member_bot(&id(), "alice", "alice")
        .await
        .unwrap();
    bed.manager.sync_member_bot_home(&alice).await.unwrap();
    assert_eq!(bed.docker.creations(&container("alice")), count);
    assert_eq!(
        bed.manager
            .store
            .bots()
            .entry(&id(), "alice")
            .unwrap()
            .unwrap(),
        original
    );
    let role = bed.roles.get("family").unwrap();
    let mut permissions = role.permissions;
    permissions.ai = AiLevel::ReadOnly;
    bed.roles
        .update(
            "family",
            &RoleDraft {
                name: "Family".into(),
                permissions: permissions.clone(),
            },
        )
        .unwrap();
    bed.manager.sync_member_bot_home(&alice).await.unwrap();
    assert_eq!(mount(&bed.docker, "alice")["ReadOnly"], true);
    assert_eq!(bed.docker.creations(&container("alice")), count + 1);
    assert_eq!(bed.docker.creations(&container("bob")), 1);
    permissions.private_bot = false;
    bed.roles
        .update(
            "family",
            &RoleDraft {
                name: "Family".into(),
                permissions,
            },
        )
        .unwrap();
    assert!(bed.manager.sync_member_bot_home(&alice).await.is_err());
    assert!(!bed.docker.containers().contains(&container("alice")));
}

#[tokio::test]
async fn failed_creation_reuses_account_and_revokes_orphan_devices() {
    let bed = Bed::start().await;
    bed.docker
        .fail_next_create(&container("alice"), "injected create failure");
    assert!(
        bed.manager
            .ensure_member_bot(&id(), "alice", "alice")
            .await
            .is_err()
    );
    let partial = bed
        .manager
        .store
        .bots()
        .entry(&id(), "alice")
        .unwrap()
        .unwrap();
    assert!(partial.device_id.is_none());
    bed.manager
        .ensure_member_bot(&id(), "alice", "alice")
        .await
        .unwrap();
    let ready = bed
        .manager
        .store
        .bots()
        .entry(&id(), "alice")
        .unwrap()
        .unwrap();
    assert_eq!(ready.account, partial.account);
    assert_eq!(ready.bus_token, partial.bus_token);
    assert_eq!(
        bed.matrix
            .state()
            .tokens
            .values()
            .filter(|login| login.0 == ready.account.user_id)
            .count(),
        1
    );
    assert!(
        bed.manager
            .ensure_member_bot(&id(), "alice", "bob")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn removal_retries_data_cleanup_and_checks_keeper_before_side_effects() {
    let bed = Bed::start().await;
    let reference = bed
        .manager
        .ensure_member_bot(&id(), "alice", "alice")
        .await
        .unwrap();
    assert!(
        bed.manager
            .remove_member_bot(&reference, "bob", false)
            .await
            .is_err()
    );
    assert!(bed.docker.containers().contains(&container("alice")));
    let data = bed.dir.path().join("apps/app-test/bots/alice");
    std::fs::remove_dir_all(&data).unwrap();
    std::fs::write(&data, b"blocks directory removal").unwrap();
    assert!(
        bed.manager
            .remove_member_bot(&reference, "alice", false)
            .await
            .is_err()
    );
    assert!(
        bed.manager
            .store
            .bots()
            .entry(&id(), "alice")
            .unwrap()
            .is_some()
    );
    std::fs::remove_file(&data).unwrap();
    std::fs::create_dir(&data).unwrap();
    bed.manager
        .remove_member_bot(&reference, "alice", false)
        .await
        .unwrap();
    assert!(!data.exists());
    assert!(
        bed.manager
            .store
            .bots()
            .entry(&id(), "alice")
            .unwrap()
            .is_none()
    );
    std::fs::create_dir(&data).unwrap();
    bed.manager
        .remove_member_bot(&reference, "alice", false)
        .await
        .unwrap();
    assert!(!data.exists());
}

#[test]
fn home_mounts_reject_overlapping_targets_and_unknown_keepers() {
    let text = compose("echo").replace("${WONTON_BOT_DATA}:/data", "${WONTON_BOT_DATA}:/wonton");
    let package = PluginPackage::parse(&text, &super::langs()).unwrap();
    assert!(render(&package, &dry_run_context(&package), &super::langs()).is_err());
    let package = PluginPackage::parse(&compose("echo"), &super::langs()).unwrap();
    let mut ctx = dry_run_context(&package);
    ctx.homes.clear();
    assert!(render(&package, &ctx, &super::langs()).is_err());
}

#[tokio::test]
async fn household_bot_cannot_get_a_private_space() {
    let bed = Bed::start().await;
    assert!(
        bed.manager
            .add_bot(&id(), "family", BotKeeper::Household)
            .await
            .is_err()
    );
    assert!(
        bed.manager
            .store
            .bots()
            .entry(&id(), "family")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn keep_data_removal_keeps_files_but_revokes_the_device_and_bus_identity() {
    let bed = Bed::start().await;
    let reference = bed
        .manager
        .ensure_member_bot(&id(), "alice", "alice")
        .await
        .unwrap();
    let entry = bed
        .manager
        .store
        .bots()
        .entry(&id(), "alice")
        .unwrap()
        .unwrap();
    let data = bed.dir.path().join("apps/app-test/bots/alice");
    std::fs::write(data.join("memory"), b"keep me").unwrap();
    bed.manager
        .remove_member_bot(&reference, "alice", true)
        .await
        .unwrap();
    bed.manager
        .remove_member_bot(&reference, "alice", true)
        .await
        .unwrap();
    assert_eq!(std::fs::read(data.join("memory")).unwrap(), b"keep me");
    assert!(bed.manager.bot_by_token(&id(), &entry.bus_token).is_none());
    assert!(
        !bed.matrix
            .state()
            .tokens
            .values()
            .any(|login| login.0 == entry.account.user_id)
    );
    assert!(!bed.matrix.state().user(&entry.account.user_id).deactivated);
    assert!(
        bed.manager
            .remove_member_bot(&reference, "bob", false)
            .await
            .is_err()
    );
    bed.manager
        .remove_member_bot(&reference, "alice", false)
        .await
        .unwrap();
    assert!(!data.exists());
    assert!(bed.matrix.state().user(&entry.account.user_id).deactivated);
}

#[tokio::test]
async fn unknown_ai_is_read_only_and_roles_can_be_replaced_after_gateway_construction() {
    let bed = Bed::start().await;
    let replacement = Arc::new(Roles::initial());
    let mut permissions = replacement.get("family").unwrap().permissions;
    permissions.ai = AiLevel::Other;
    replacement
        .update(
            "family",
            &RoleDraft {
                name: "Family".into(),
                permissions,
            },
        )
        .unwrap();
    bed.manager.attach_person_roles(replacement);
    bed.manager
        .ensure_member_bot(&id(), "alice", "alice")
        .await
        .unwrap();
    assert_eq!(mount(&bed.docker, "alice")["ReadOnly"], true);
}

#[tokio::test]
async fn reserved_matrix_account_survives_a_failed_provisioning_checkpoint() {
    let bed = Bed::start().await;
    let matrix = bed.manager.matrix().unwrap();
    let taken = std::collections::BTreeSet::new();
    let spec = crate::matrix::BotSpec {
        localpart: "reserved",
        taken: &taken,
        previous: None,
        device_name: "test",
        display_name: "Reserved",
        avatar: None,
        owner: "alice",
    };
    let account = matrix.reserve_bot_account(&spec).await.unwrap();
    assert!(!matrix.user_exists(&account.user_id).await.unwrap());
    let spec = crate::matrix::BotSpec {
        previous: Some(&account),
        ..spec
    };
    assert!(
        matrix
            .provision_bot_checkpointed(&spec, |_| {
                Err(crate::matrix::MatrixError::Invalid(
                    "injected persistence failure".into(),
                ))
            })
            .await
            .is_err()
    );
    assert!(matrix.user_exists(&account.user_id).await.unwrap());
    assert!(matrix.devices(&account.user_id).await.unwrap().is_empty());
    let retry = matrix.provision_bot(&spec).await.unwrap();
    assert_eq!(retry.account.user_id, account.user_id);
    assert!(!retry.account.home_room.is_empty());
    assert!(!matrix.user_exists("@reserved-2:test").await.unwrap());
    matrix
        .delete_device(&retry.account.user_id, &retry.device_id)
        .await
        .unwrap();
}

#[test]
fn rendering_multiple_bots_never_reuses_the_first_bots_identity_or_home() {
    let package = PluginPackage::parse(&compose("echo"), &super::langs()).unwrap();
    let mut ctx = dry_run_context(&package);
    let mut alice = ctx.bots["default"].clone();
    alice.matrix.as_mut().unwrap().user_id = "@alice-bot:test".into();
    ctx.bots.insert("alice".into(), alice);
    ctx.homes.insert(
        "alice".into(),
        super::super::private_homes::HomeMount {
            path: "/wonton-dry-run/core/home/alice".into(),
            read_only: true,
        },
    );
    let plan = render(&package, &ctx, &super::langs()).unwrap();
    for bot in ["default", "alice"] {
        let container = plan
            .containers
            .iter()
            .find(|container| container.bot.as_deref() == Some(bot))
            .unwrap();
        let mounts = container
            .body
            .host_config
            .as_ref()
            .unwrap()
            .mounts
            .as_ref()
            .unwrap();
        let home = mounts
            .iter()
            .find(|mount| mount.target.as_deref() == Some("/wonton/home"))
            .unwrap();
        assert_eq!(home.source.as_deref(), ctx.homes[bot].path.to_str());
        assert_eq!(home.read_only, Some(ctx.homes[bot].read_only));
        let data = mounts
            .iter()
            .find(|mount| mount.target.as_deref() == Some("/data"))
            .unwrap();
        assert_eq!(
            data.source.as_deref(),
            ctx.app_data.join("bots").join(bot).to_str()
        );
        assert!(container.body.env.as_ref().unwrap().contains(&format!(
            "USER={}",
            ctx.bots[bot].matrix.as_ref().unwrap().user_id
        )));
    }
}

#[test]
fn manifest_read_permission_forces_read_only_even_for_the_owner() {
    let text = compose("echo").replace("storage.home:write", "storage.home:read");
    let package = PluginPackage::parse(&text, &super::langs()).unwrap();
    let plan = render(&package, &dry_run_context(&package), &super::langs()).unwrap();
    let bot = plan
        .containers
        .iter()
        .find(|container| container.bot.is_some())
        .unwrap();
    let mounts = bot
        .body
        .host_config
        .as_ref()
        .unwrap()
        .mounts
        .as_ref()
        .unwrap();
    assert_eq!(
        mounts
            .iter()
            .find(|mount| mount.target.as_deref() == Some("/wonton/home"))
            .unwrap()
            .read_only,
        Some(true)
    );
}

#[tokio::test]
async fn first_matrix_room_failure_keeps_the_reserved_account_for_retry() {
    let bed = Bed::start().await;
    bed.matrix.state().refuse_rooms = true;
    assert!(
        bed.manager
            .ensure_member_bot(&id(), "alice", "alice")
            .await
            .is_err()
    );
    assert!(bed.manager.bot_account(&id(), "alice").is_none());
    let reserved = bed
        .manager
        .store
        .bots()
        .pending(&id())
        .unwrap()
        .remove("alice")
        .unwrap();
    assert!(reserved.home_room.is_none());
    assert!(
        bed.manager
            .matrix()
            .unwrap()
            .devices(&reserved.user_id)
            .await
            .unwrap()
            .is_empty()
    );
    bed.matrix.state().refuse_rooms = false;
    bed.manager
        .ensure_member_bot(&id(), "alice", "alice")
        .await
        .unwrap();
    let ready = bed
        .manager
        .store
        .bots()
        .entry(&id(), "alice")
        .unwrap()
        .unwrap();
    assert_eq!(ready.account.user_id, reserved.user_id);
    assert_eq!(ready.bus_token, reserved.bus_token);
    assert!(!ready.account.home_room.is_empty());
    assert!(
        !bed.manager
            .matrix()
            .unwrap()
            .user_exists("@test-alice-2:test")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn missing_room_checkpoint_recovers_the_marked_room_instead_of_creating_another() {
    let bed = Bed::start().await;
    let matrix = bed.manager.matrix().unwrap();
    let taken = std::collections::BTreeSet::new();
    let spec = crate::matrix::BotSpec {
        localpart: "reserved-room",
        taken: &taken,
        previous: None,
        device_name: "test",
        display_name: "Reserved",
        avatar: None,
        owner: "alice",
    };
    let account = matrix.reserve_bot_account(&spec).await.unwrap();
    let spec = crate::matrix::BotSpec {
        previous: Some(&account),
        ..spec
    };
    let first = matrix.provision_bot(&spec).await.unwrap();
    matrix
        .delete_device(&first.account.user_id, &first.device_id)
        .await
        .unwrap();
    let rooms = bed.matrix.state().rooms.len();
    let recovered = matrix.provision_bot(&spec).await.unwrap();
    assert_eq!(recovered.account, first.account);
    assert_eq!(bed.matrix.state().rooms.len(), rooms);
    matrix
        .delete_device(&recovered.account.user_id, &recovered.device_id)
        .await
        .unwrap();
}
