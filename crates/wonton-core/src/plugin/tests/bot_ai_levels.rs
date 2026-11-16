//! Bot 容器里的 AI 级别（`WONTON_BOT_AI_LEVEL`）和共享文件夹只读挂载（ADR-0088 第 5、8 节）：
//! 归属人“AI 动手”是只读的私人 Bot，共享文件夹一律只读挂载、环境变量是他的级别；共享 Bot 的环境
//! 变量是能用人里最低的级别，挂载不按人强制。角色改了以后经渲染摘要的差异重建容器。

use std::sync::Arc;

use serde_json::Value;
use wonton_protocol::{AiLevel, MemberStatus, NewSharedBot, Permissions, RoleDraft};

use super::super::fake_docker::FakeDocker;
use super::super::manifest::{Permission, PluginId};
use super::manager::manager_in_container;
use crate::members::{MemberRecord, Members};
use crate::roles::Roles;

fn compose(image: &str) -> String {
    super::package(
        &format!(
            "  bot:\n    image: {image}\n    command: ['sleep', 'infinity']\n    volumes: \
             ['${{WONTON_BOT_DATA}}:/data', '${{WONTON_SHARE_PHOTOS}}:/photos']\n    \
             environment:\n      USER: ${{WONTON_MATRIX_USER_ID}}\n      AI: \
             ${{WONTON_BOT_AI_LEVEL}}\n  web:\n    image: {image}\n    command: ['sleep', \
             'infinity']\n    volumes: ['${{WONTON_SHARE_PHOTOS}}:/photos']"
        ),
        "  agent: {protocol: matrix}\n  bots: {services: [bot]}\n  permissions: [matrix.bot, \
         storage.share.photos:write]",
    )
    .replace("kind: app", "kind: agent")
}

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}

fn permissions() -> Vec<Permission> {
    vec![
        Permission::MatrixBot,
        "storage.share.photos:write".parse().unwrap(),
    ]
}

fn bot_container(bot: &str) -> String {
    super::super::names::Instance::default().bot_container("app-test", "bot", bot)
}

fn share_mount(docker: &FakeDocker, container: &str) -> Value {
    docker.created(container).unwrap()["HostConfig"]["Mounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mount| mount["Target"] == "/photos")
        .unwrap()
        .clone()
}

fn env(docker: &FakeDocker, container: &str) -> std::collections::BTreeMap<String, String> {
    docker
        .env(container)
        .iter()
        .filter_map(|item| item.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

struct Bed {
    /// 临时目录只在字段里占位：核心数据目录在这里面，删了路径就没了。
    _dir: tempfile::TempDir,
    docker: FakeDocker,
    /// 假 Tuwunel 要留着：Bot 的身份核对、令牌都问它。
    _matrix: crate::matrix::fake::Fake,
    manager: super::super::manager::PluginManager,
    roles: Arc<Roles>,
    /// 只读角色（“用共享 Bot”为“能”）的 ID。
    viewer: String,
}

impl Bed {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (docker, engine) = FakeDocker::start().await;
        let (fake_matrix, cfg) = crate::matrix::fake::start().await;
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
        // 只读但能用共享 Bot 的角色：比“朋友”模板多“用共享 Bot”。
        let viewer = roles
            .create(&RoleDraft {
                name: "Viewer".into(),
                permissions: Permissions {
                    shared_bots: true,
                    private_bot: true,
                    ..Permissions::strictest()
                },
            })
            .unwrap()
            .id;
        for (username, role_id) in [("alice", viewer.as_str()), ("bob", "family")] {
            let mut member = MemberRecord::for_test(username);
            member.status = MemberStatus::Joined;
            member.role_id = role_id.to_owned();
            members.invite(member).unwrap();
            members.mark_joined(username).unwrap();
        }
        manager.attach_person_state(members, roles.clone());
        manager
            .install(&compose("echo"), &permissions(), super::langs())
            .await
            .unwrap();
        Self {
            _dir: dir,
            docker,
            _matrix: fake_matrix,
            manager,
            roles,
            viewer,
        }
    }
}

#[tokio::test]
async fn a_read_only_keeper_mounts_shares_read_only_and_reports_the_level() {
    let bed = Bed::start().await;
    bed.manager
        .ensure_member_bot(&id(), "alice", "alice")
        .await
        .unwrap();
    // 归属人只读：环境变量是他的级别，共享文件夹挂载一律只读（插件声明了写权限也一样）。
    assert_eq!(env(&bed.docker, &bot_container("alice"))["AI"], "read_only");
    assert_eq!(
        share_mount(&bed.docker, &bot_container("alice"))["ReadOnly"],
        true
    );
    // 全插件一份的服务不属于任何 Bot：挂载照清单的写权限，不按人强制。
    let web = super::super::names::container_name("app-test", "web");
    assert_eq!(share_mount(&bed.docker, &web)["ReadOnly"], false);
    // 默认 Bot 归主人，主人不受角色约束。
    assert_eq!(
        env(&bed.docker, &bot_container("default"))["AI"],
        "self_confirms"
    );
    assert_eq!(
        share_mount(&bed.docker, &bot_container("default"))["ReadOnly"],
        false
    );
}

#[tokio::test]
async fn a_self_confirms_keeper_mounts_shares_writable() {
    let bed = Bed::start().await;
    bed.manager
        .ensure_member_bot(&id(), "bob", "bob")
        .await
        .unwrap();
    assert_eq!(
        env(&bed.docker, &bot_container("bob"))["AI"],
        "self_confirms"
    );
    assert_eq!(
        share_mount(&bed.docker, &bot_container("bob"))["ReadOnly"],
        false
    );
}

#[tokio::test]
async fn a_shared_bot_reports_the_lowest_level_of_its_users() {
    let bed = Bed::start().await;
    bed.manager
        .create_shared_bot(&NewSharedBot {
            plugin_id: id().to_string(),
            bot: "family".to_owned(),
        })
        .await
        .unwrap();
    // alice（只读）和 bob（改动自己确认）都能用它：环境变量取最低的只读。
    assert_eq!(
        env(&bed.docker, &bot_container("family"))["AI"],
        "read_only"
    );
    // 共享 Bot 没有归属人：挂载不按人强制，照清单的写权限。
    assert_eq!(
        share_mount(&bed.docker, &bot_container("family"))["ReadOnly"],
        false
    );
}

#[tokio::test]
async fn a_role_change_rebuilds_the_private_bot_with_the_new_level() {
    let bed = Bed::start().await;
    let reference = bed
        .manager
        .ensure_member_bot(&id(), "alice", "alice")
        .await
        .unwrap();
    assert_eq!(env(&bed.docker, &bot_container("alice"))["AI"], "read_only");
    assert_eq!(
        share_mount(&bed.docker, &bot_container("alice"))["ReadOnly"],
        true
    );
    let count = bed.docker.creations(&bot_container("alice"));
    let mut permissions = bed.roles.get(&bed.viewer).unwrap().permissions;
    permissions.ai = AiLevel::SelfConfirms;
    bed.roles
        .update(
            &bed.viewer,
            &RoleDraft {
                name: "Viewer".into(),
                permissions,
            },
        )
        .unwrap();
    bed.manager.sync_member_bot_home(&reference).await.unwrap();
    assert_eq!(bed.docker.creations(&bot_container("alice")), count + 1);
    assert_eq!(
        env(&bed.docker, &bot_container("alice"))["AI"],
        "self_confirms"
    );
    assert_eq!(
        share_mount(&bed.docker, &bot_container("alice"))["ReadOnly"],
        false
    );
    // 改回去同样重建，不留在旧的级别上。
    let mut permissions = bed.roles.get(&bed.viewer).unwrap().permissions;
    permissions.ai = AiLevel::ReadOnly;
    bed.roles
        .update(
            &bed.viewer,
            &RoleDraft {
                name: "Viewer".into(),
                permissions,
            },
        )
        .unwrap();
    bed.manager.sync_member_bot_home(&reference).await.unwrap();
    assert_eq!(bed.docker.creations(&bot_container("alice")), count + 2);
    assert_eq!(env(&bed.docker, &bot_container("alice"))["AI"], "read_only");
    assert_eq!(
        share_mount(&bed.docker, &bot_container("alice"))["ReadOnly"],
        true
    );
}
