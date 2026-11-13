#[path = "family_private_files_tests.rs"]
mod files;
#[path = "family_private_permissions_tests.rs"]
mod permissions;

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::Arc;
use std::time::Duration;

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use wonton_protocol::{BotKeeper, BotRef, MEMBER_SYNC_PATH, MemberStatus};

use crate::config::PublicUrls;
use crate::gateway::fakes::loopback_tunnel;
use crate::gateway::family_tests::{bearer, send_as};
use crate::gateway::tests::{CORE, Reply, TOKEN, owner};
use crate::gateway::{Gateway, router};
use crate::http::HttpClient;
use crate::matrix::{Matrix, fake};
use crate::members::{MemberRecord, Members};
use crate::owner::OwnerStore;
use crate::plugin::bots::BotStore;
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{PluginId, PluginManager};
use crate::roles::Roles;
use crate::tunnel::Tunnel;

const AGENT: &str = "app.echo";
const PACKAGE: &str = r#"
services:
  bot:
    image: echo
    environment:
      TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}
      ROOM: ${WONTON_MATRIX_HOME_ROOM}
    volumes: ["${WONTON_BOT_DATA}:/data"]
x-wonton:
  spec: '0.5'
  id: app.echo
  version: 1.0.0
  kind: agent
  title: {zh: 回声, en: Echo}
  description: {zh: 回声, en: Echo}
  agent: {protocol: matrix}
  bots: {services: [bot]}
  permissions: [matrix.bot, storage.home:write]
"#;

struct Bed {
    dir: tempfile::TempDir,
    docker: FakeDocker,
    engine: Engine,
    fake: fake::Fake,
    matrix: Arc<Matrix>,
    gateway: Arc<Gateway>,
}

impl Bed {
    async fn start() -> Self {
        Self::start_custom(None, true).await
    }

    /// `owner` 传 Some 时用它当数据属主（测别人属主的行为）；`files` 传 false 时不装 Files 服务：
    /// 装服务会以数据属主建 core/files，非 root 下 chown 给别的用户必 EPERM，恰是被测行为。
    async fn start_custom(owner: Option<crate::plugin::hostdir::Owner>, files: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (docker, engine) = FakeDocker::start().await;
        let (fake, config) = fake::start().await;
        let matrix = Arc::new(
            Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
                .await
                .unwrap(),
        );
        let (tunnel, members) = loopback_tunnel(dir.path()).await;
        let ids = fs::metadata(dir.path()).unwrap();
        let gateway = Self::gateway(
            dir.path(),
            engine.clone(),
            matrix.clone(),
            tunnel,
            members,
            owner.unwrap_or(crate::plugin::hostdir::Owner {
                uid: ids.uid(),
                gid: ids.gid(),
            }),
            files,
        );
        fs::write(
            dir.path().join("core/onboarding.json"),
            json!({
                "step": "finished", "primary_agent": AGENT, "replaced_agent_asked": false,
            })
            .to_string(),
        )
        .unwrap();
        let permissions = vec![
            "matrix.bot".parse().unwrap(),
            "storage.home:write".parse().unwrap(),
        ];
        gateway
            .plugins
            .install(PACKAGE, &permissions, Languages::default())
            .await
            .unwrap();
        Self {
            dir,
            docker,
            engine,
            fake,
            matrix,
            gateway,
        }
    }

    fn gateway(
        dir: &std::path::Path,
        engine: Engine,
        matrix: Arc<Matrix>,
        tunnel: Tunnel,
        members: Arc<Members>,
        data_owner: crate::plugin::hostdir::Owner,
        files: bool,
    ) -> Arc<Gateway> {
        let profile = OwnerStore::for_test(dir, &owner());
        let public = PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: 18480,
        };
        let config = LifecycleConfig {
            public: public.clone(),
            data_dir: dir.to_owned(),
            apps_dir: dir.join("apps"),
            shares_dir: dir.join("shares"),
            relay_dir: dir.join("relay"),
            core_container: None,
            tz: "UTC".into(),
            puid: data_owner.uid,
            pgid: data_owner.gid,
            install_timeout: Duration::from_secs(5),
            owner: "owner".into(),
            profile: profile.clone(),
            gateway_port: 8080,
            plugin_slice: None,
            instance: Default::default(),
        };
        let plugins = Arc::new(
            PluginManager::new(
                engine,
                Store::open(dir.join("plugins")).unwrap(),
                HttpClient::new(),
                None,
                config,
                ImageWorker::in_process(),
            )
            .with_matrix(Some(matrix.clone())),
        );
        let gateway = Gateway::new(
            public,
            profile,
            TOKEN.into(),
            members,
            plugins,
            None,
            HttpClient::new(),
            tunnel,
        )
        .with_roles(Arc::new(Roles::open(dir.join("roles.json")).unwrap()))
        .with_matrix(Some(matrix));
        let files = files.then(|| files::service(&gateway));
        Arc::new(gateway.with_files(files))
    }

    async fn call(&self, method: Method, path: &str, body: Value) -> Reply {
        send_as(
            &router(self.gateway.clone()),
            None,
            method,
            CORE,
            path,
            &[
                ("authorization", &bearer()),
                ("content-type", "application/json"),
            ],
            &body.to_string(),
        )
        .await
    }

    fn pending(&self, username: &str, private_bot: bool) -> (String, iroh::EndpointId) {
        self.gateway
            .members
            .invite(MemberRecord {
                role_id: "family".into(),
                wants_private_bot: private_bot,
                ..MemberRecord::for_test(username)
            })
            .unwrap();
        let code = self.gateway.members.issue_pairing_code(username).unwrap();
        let device = SecretKey::generate().public();
        self.gateway
            .members
            .request_join(
                code.secret.expose(),
                device,
                "Phone",
                &self.gateway.tunnel.devices,
            )
            .unwrap();
        let id = self
            .gateway
            .members
            .join_requests()
            .iter()
            .find(|request| request.username == username)
            .unwrap()
            .id
            .clone();
        (id, device)
    }

    async fn approve(&self, id: &str) -> Reply {
        self.call(
            Method::POST,
            &wonton_protocol::join_request_approve_path(id),
            Value::Null,
        )
        .await
    }

    async fn retry(&self) -> Reply {
        self.call(Method::POST, MEMBER_SYNC_PATH, Value::Null).await
    }

    fn bot(&self, username: &str) -> BotRef {
        self.gateway
            .members
            .get(username)
            .unwrap()
            .private_bot
            .unwrap()
    }

    fn home(&self, username: &str) -> std::path::PathBuf {
        crate::private_home::path(self.dir.path(), username).unwrap()
    }

    async fn reopen(&mut self) {
        let members = Arc::new(Members::open(self.dir.path().join("members.json")).unwrap());
        let ids = fs::metadata(self.dir.path()).unwrap();
        self.gateway = Self::gateway(
            self.dir.path(),
            self.engine.clone(),
            self.matrix.clone(),
            self.gateway.tunnel.clone(),
            members,
            crate::plugin::hostdir::Owner {
                uid: ids.uid(),
                gid: ids.gid(),
            },
            true,
        );
        self.gateway.plugins.restore().await.unwrap();
    }
}

#[tokio::test]
async fn approval_builds_private_bot_home_and_exposes_only_management_metadata() {
    let bed = Bed::start().await;
    let (id, device) = bed.pending("alice_smith", true);
    assert!(!bed.home("alice_smith").exists());
    let reply = bed.approve(&id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    assert_eq!(
        bed.gateway
            .tunnel
            .devices
            .get(&device)
            .unwrap()
            .user
            .as_deref(),
        Some("alice_smith")
    );
    let bot = bed.bot("alice_smith");
    let stored = BotStore::new(bed.dir.path().join("plugins/bots"))
        .load(&PluginId::parse(AGENT).unwrap())
        .unwrap();
    let entry = &stored.bots[&bot.bot];
    assert_eq!(
        entry.keeper,
        BotKeeper::User {
            username: "alice_smith".into()
        }
    );
    assert_eq!(
        fs::metadata(bed.home("alice_smith"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    {
        let state = bed.fake.state();
        let room = &state.rooms[&entry.account.home_room];
        assert!(room.members.contains("@alice_smith:test"));
        assert!(!room.members.contains("@owner:test"));
    }
    let reply = bed
        .call(Method::GET, wonton_protocol::MEMBERS_PATH, Value::Null)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["members"][0]["private_bot"], json!(bot));
    let writes = bed.docker.writes();
    let reply = bed.retry().await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    assert_eq!(bed.bot("alice_smith"), bot);
    assert_eq!(
        bed.docker.writes(),
        writes,
        "a completed setup must not restart healthy bots"
    );
}

#[tokio::test]
async fn directory_failure_is_visible_and_retry_resumes_the_saved_approval() {
    let bed = Bed::start().await;
    fs::write(bed.home("alice"), b"not a directory").unwrap();
    let (id, _) = bed.pending("alice", false);
    let reply = bed.approve(&id).await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert_eq!(reply.error_kind(), Some("storage"));
    assert_eq!(
        bed.gateway.members.get("alice").unwrap().status,
        MemberStatus::Joined
    );
    fs::remove_file(bed.home("alice")).unwrap();
    let reply = bed.retry().await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    assert!(bed.home("alice").is_dir());
    assert!(
        bed.gateway
            .members
            .get("alice")
            .unwrap()
            .private_bot
            .is_none()
    );
}

/// 改属主被拒（EPERM/EACCES）不是笼统的存储错误：对外是“权限缺失”，详情写清目录和要求的属主。
/// 目标属主取当前进程的下一个 uid：非 root（CI runner、开发机都是）改属主被拒，正好走 EPERM；
/// root 下 chown 会成功，那批断言不适用，按 hostdir 属主测试的同一处理改为断言批准完成。
#[tokio::test]
async fn a_home_whose_ownership_cannot_be_fixed_is_reported_as_missing_permission() {
    let bed = Bed::start().await;
    let ids = fs::metadata(bed.dir.path()).unwrap();
    let foreign = crate::plugin::hostdir::Owner {
        uid: ids.uid() + 1,
        gid: ids.gid(),
    };
    let members = Arc::new(Members::open(bed.dir.path().join("members.json")).unwrap());
    let gateway = Bed::gateway(
        bed.dir.path(),
        bed.engine.clone(),
        bed.matrix.clone(),
        bed.gateway.tunnel.clone(),
        members,
        foreign,
        false,
    );
    let id = {
        gateway
            .members
            .invite(MemberRecord {
                role_id: "family".into(),
                wants_private_bot: false,
                ..MemberRecord::for_test("alice")
            })
            .unwrap();
        let code = gateway.members.issue_pairing_code("alice").unwrap();
        let device = SecretKey::generate().public();
        gateway
            .members
            .request_join(
                code.secret.expose(),
                device,
                "Phone",
                &gateway.tunnel.devices,
            )
            .unwrap();
        gateway
            .members
            .join_requests()
            .iter()
            .find(|request| request.username == "alice")
            .unwrap()
            .id
            .clone()
    };
    let reply = send_as(
        &router(gateway.clone()),
        None,
        Method::POST,
        CORE,
        &wonton_protocol::join_request_approve_path(&id),
        &[
            ("authorization", &bearer()),
            ("content-type", "application/json"),
        ],
        "",
    )
    .await;
    if reply.status == StatusCode::NO_CONTENT {
        // root 下改属主会成功：家目录按配置交给属主，批准照常完成。
        assert!(bed.home("alice").is_dir());
    } else {
        assert_eq!(
            reply.status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{}",
            reply.text()
        );
        assert_eq!(
            reply.error_kind(),
            Some("permission_denied"),
            "{}",
            reply.text()
        );
        assert!(reply.text().contains("core/home/alice"), "{}", reply.text());
        assert!(
            reply
                .text()
                .contains(&format!("{}:{}", foreign.uid, foreign.gid)),
            "{}",
            reply.text()
        );
    }
}

#[tokio::test]
async fn failed_bot_start_is_not_published_and_retry_reuses_its_reserved_identity() {
    let bed = Bed::start().await;
    let (id, _) = bed.pending("alice", true);
    bed.docker.fail_starts(Some("deliberate start failure"));
    let reply = bed.approve(&id).await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert!(
        bed.gateway
            .members
            .get("alice")
            .unwrap()
            .private_bot
            .is_none()
    );
    let reserved = bed.gateway.members.lifecycle("alice").bot.unwrap();
    let account = bed
        .gateway
        .plugins
        .bot_account(&PluginId::parse(AGENT).unwrap(), &reserved.bot)
        .unwrap();
    bed.docker.fail_starts(None);
    let reply = bed.retry().await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    assert_eq!(bed.bot("alice"), reserved);
    let recovered = bed
        .gateway
        .plugins
        .bot_account(&PluginId::parse(AGENT).unwrap(), &reserved.bot)
        .unwrap();
    assert_eq!(recovered.user_id, account.user_id);
    assert_eq!(bed.fake.state().devices(&recovered.user_id).len(), 1);
}

#[tokio::test]
async fn removal_respects_independent_home_and_bot_data_choices() {
    let bed = Bed::start().await;
    for (username, delete_home, keep_bot_data) in [("alice", false, true), ("bob", true, false)] {
        let (id, _) = bed.pending(username, true);
        let reply = bed.approve(&id).await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
        let bot = bed.bot(username);
        let data = bed.dir.path().join("apps/app-echo/bots").join(&bot.bot);
        fs::write(data.join("memory"), b"private bot data").unwrap();
        fs::write(bed.home(username).join("file"), b"private file").unwrap();
        let reply = bed
            .call(
                Method::POST,
                &wonton_protocol::member_removal_path(username),
                json!({"delete_home": delete_home, "keep_bot_data": keep_bot_data}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        assert_eq!(bed.home(username).exists(), !delete_home);
        assert_eq!(data.join("memory").exists(), keep_bot_data);
        assert!(
            bed.gateway
                .members
                .get(username)
                .unwrap()
                .private_bot
                .is_none()
        );
        assert!(
            bed.fake
                .state()
                .user(&format!("@{username}:test"))
                .deactivated
        );
    }
}

#[tokio::test]
async fn failed_removal_survives_restart_and_uses_the_recorded_choices() {
    let mut bed = Bed::start().await;
    let (id, _) = bed.pending("alice", true);
    let reply = bed.approve(&id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let home = bed.home("alice");
    let saved = bed.dir.path().join("saved-home");
    fs::rename(&home, &saved).unwrap();
    std::os::unix::fs::symlink(&saved, &home).unwrap();
    let reply = bed
        .call(
            Method::POST,
            &wonton_protocol::member_removal_path("alice"),
            json!({"delete_home": true, "keep_bot_data": true}),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    fs::remove_file(&home).unwrap();
    fs::rename(&saved, &home).unwrap();
    bed.reopen().await;
    bed.gateway.sync_members().await.unwrap();
    assert!(!home.exists());
    assert!(bed.gateway.members.lifecycle("alice").removal.is_none());
    assert!(
        bed.gateway
            .members
            .get("alice")
            .unwrap()
            .private_bot
            .is_none()
    );
}

#[tokio::test]
async fn missing_primary_agent_is_reported_without_guessing_an_installed_agent() {
    let bed = Bed::start().await;
    fs::remove_file(bed.dir.path().join("core/onboarding.json")).unwrap();
    let (id, _) = bed.pending("alice", true);
    let reply = bed.approve(&id).await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert_eq!(reply.error_kind(), Some("not_installed"));
    assert!(
        bed.gateway
            .members
            .get("alice")
            .unwrap()
            .private_bot
            .is_none()
    );
    assert!(bed.gateway.members.lifecycle("alice").bot.is_none());
    fs::write(
        bed.dir.path().join("core/onboarding.json"),
        json!({
            "step": "finished", "primary_agent": AGENT, "replaced_agent_asked": false,
        })
        .to_string(),
    )
    .unwrap();
    let reply = bed.retry().await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    assert_eq!(bed.bot("alice").plugin_id, AGENT);
}

#[tokio::test]
async fn member_administrators_see_bot_references_but_ordinary_members_do_not() {
    let bed = Bed::start().await;
    let (id, alice) = bed.pending("alice", true);
    let reply = bed.approve(&id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let (id, admin) = bed.pending("admin", false);
    bed.gateway.members.set_role("admin", "admin").unwrap();
    let reply = bed.approve(&id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let app = router(bed.gateway.clone());
    let reply = send_as(
        &app,
        Some(admin),
        Method::GET,
        CORE,
        wonton_protocol::MEMBERS_PATH,
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let list = reply.json();
    let alice_record = list["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["username"] == "alice")
        .unwrap();
    assert_eq!(alice_record["private_bot"], json!(bed.bot("alice")));
    let reply = send_as(
        &app,
        Some(alice),
        Method::GET,
        CORE,
        wonton_protocol::MEMBERS_PATH,
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.json()["missing"], "manage_members");
}
