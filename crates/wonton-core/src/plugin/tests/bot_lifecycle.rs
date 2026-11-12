//! Bot 插件在 Matrix 上的副作用：在假 Docker 和假 Tuwunel 上验证安装、卸载、删除数据、回滚和核心重启。

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::store::Store;
use super::super::{PluginId, PluginManager, PluginStatus};
use super::manager::manager_in_container;
use super::{env_of, example, package};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{Fake, start};
use crate::plugin::render::dry_run;

/// 一个回声 Bot：唯一的服务拿到 Bot 的全部 Matrix 变量。
fn echo() -> String {
    package(
        "  bot:\n    image: echo\n    environment:\n      \
         HS: ${WONTON_MATRIX_HOMESERVER}\n      USER: ${WONTON_MATRIX_USER_ID}\n      \
         TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}\n      OWNER: ${WONTON_MATRIX_OWNER}\n      \
         ROOM: ${WONTON_MATRIX_HOME_ROOM}",
        "  bot: {display_name: {zh: 回声, en: Echo}}\n  permissions: [matrix.bot]",
    )
    .replace("id: app.test", "id: app.echo")
}

fn granted() -> Vec<super::super::Permission> {
    vec!["matrix.bot".parse().unwrap()]
}

struct Bed {
    docker: FakeDocker,
    engine: Engine,
    tuwunel: Fake,
    matrix: Arc<Matrix>,
    dir: tempfile::TempDir,
}

impl Bed {
    async fn start() -> Self {
        let (docker, engine) = FakeDocker::start().await;
        let (tuwunel, config) = start().await;
        let dir = tempfile::tempdir().unwrap();
        let matrix = Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
            .await
            .unwrap();
        Self {
            docker,
            engine,
            tuwunel,
            matrix: Arc::new(matrix),
            dir,
        }
    }

    /// 核心（重新）启动后的插件管理器，数据目录不变。
    fn manager(&self) -> PluginManager {
        manager_in_container(self.engine.clone(), &self.dir).with_matrix(Some(self.matrix.clone()))
    }

    /// Bot 容器拿到的变量：名字 → 值。
    fn env(&self) -> std::collections::BTreeMap<String, String> {
        self.docker
            .env(&container_name("app-echo", "bot"))
            .iter()
            .filter_map(|e| e.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    fn token_valid(&self, token: &str) -> bool {
        self.tuwunel.state().tokens.contains_key(token)
    }
    fn source_address(&self) -> std::net::IpAddr {
        let body = self.docker.network("wonton-app-echo").unwrap();
        let subnet: ipnet::Ipv4Net = body["IPAM"]["Config"][0]["Subnet"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        std::net::Ipv4Addr::from(subnet.network().to_bits() + 9).into()
    }
}

fn id() -> PluginId {
    PluginId::parse("app.echo").unwrap()
}

#[tokio::test]
async fn a_bot_plugin_gets_its_own_matrix_identity_through_the_core() {
    let bed = Bed::start().await;
    let m = bed.manager();
    m.install(&echo(), &granted(), super::langs())
        .await
        .unwrap();
    let env = bed.env();
    assert_eq!(env["HS"], "http://matrix.wonton.internal:8080");
    assert_eq!(env["USER"], "@echo:test");
    assert_eq!(env["OWNER"], "@owner:test");
    assert!(bed.token_valid(&env["TOKEN"]), "令牌属于 Bot 刚登录的设备");
    {
        let state = bed.tuwunel.state();
        let room = &state.rooms[&env["ROOM"]];
        assert!(room.encrypted && room.members.contains("@owner:test"));
        let name = state.user("@echo:test").display_name.as_deref();
        assert_eq!(name, Some("回声"));
    }
    // 核心在插件网络里的别名，插件经它访问 Matrix；网关按来源地址认出这个插件。
    let aliases = bed.docker.aliases("wonton-app-echo");
    assert_eq!(aliases, ["matrix.wonton.internal"]);
    let source = m.source(bed.source_address()).unwrap();
    assert_eq!(source.id, id());
    assert!(source.permissions.contains(&granted()[0]));
}

#[tokio::test]
async fn uninstall_revokes_the_bot_and_deleting_data_deactivates_its_account() {
    let bed = Bed::start().await;
    let m = bed.manager();
    m.install(&echo(), &granted(), super::langs())
        .await
        .unwrap();
    let first = bed.env();
    let source_address = bed.source_address();
    bed.matrix
        .as_bot(&first["USER"], async |session| {
            session.set_display_name("自选名字").await
        })
        .await
        .unwrap();

    let report = m.uninstall(&id(), false, &super::langs()).await.unwrap();
    let bot = report.bot.unwrap();
    assert_eq!(
        (bot.user_id.as_str(), bot.devices_removed, bot.deactivated),
        ("@echo:test", 1, false)
    );
    assert!(
        !bed.token_valid(&first["TOKEN"]),
        "卸载后 Bot 的令牌立即失效"
    );
    assert!(m.source(source_address).is_none());

    // 不删数据重装：同一个账号、同一个主房间，新的设备。
    m.install(&echo(), &granted(), super::langs())
        .await
        .unwrap();
    let second = bed.env();
    assert_eq!(
        (&second["USER"], &second["ROOM"]),
        (&first["USER"], &first["ROOM"])
    );
    assert_ne!(second["TOKEN"], first["TOKEN"]);
    assert!(bed.token_valid(&second["TOKEN"]));
    assert_eq!(
        bed.tuwunel
            .state()
            .user(&second["USER"])
            .display_name
            .as_deref(),
        Some("自选名字")
    );

    // 删除数据：账号停用，不可恢复；再装是一个新账号和新的主房间。
    let report = m.uninstall(&id(), true, &super::langs()).await.unwrap();
    assert!(report.bot.unwrap().deactivated);
    assert!(bed.tuwunel.state().user("@echo:test").deactivated);
    let store = Store::open(bed.dir.path().join("plugins")).unwrap();
    assert_eq!(store.bots().load(&id()).unwrap(), None);
    m.install(&echo(), &granted(), super::langs())
        .await
        .unwrap();
    let third = bed.env();
    assert_eq!(third["USER"], "@echo-2:test");
    assert_ne!(third["ROOM"], first["ROOM"]);
}

#[tokio::test]
async fn a_failed_install_leaves_the_owner_no_chat_and_keeps_the_home_room_for_next_time() {
    let bed = Bed::start().await;
    let m = bed.manager();
    let bot = container_name("app-echo", "bot");
    bed.docker.set_state(
        &bot,
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    m.install(&echo(), &granted(), super::langs())
        .await
        .unwrap_err();
    let failed = bed.env();
    assert!(!bed.token_valid(&failed["TOKEN"]), "回滚吊销这次登录的设备");
    {
        let state = bed.tuwunel.state();
        assert!(state.devices("@echo:test").is_empty());
        let room = &state.rooms[&failed["ROOM"]];
        assert_eq!(room.members, BTreeSet::from(["@echo:test".to_owned()]));
        assert!(
            room.invited.is_empty(),
            "没装好的 Bot 不出现在主人的聊天列表里"
        );
    }

    bed.docker
        .set_state(&bot, json!({"Status": "running", "Running": true}));
    m.install(&echo(), &granted(), super::langs())
        .await
        .unwrap();
    assert_eq!(
        bed.env()["ROOM"],
        failed["ROOM"],
        "重试沿用上次建好的主房间"
    );
    let state = bed.tuwunel.state();
    assert_eq!(state.rooms.len(), 1);
    assert!(state.rooms[&failed["ROOM"]].members.contains("@owner:test"));
}

/// 核心在安装半路重启：上次登录的设备没人吊销，它的令牌还在残留的容器里。再装时吊销，只留这次的一台。
#[tokio::test]
async fn a_core_restart_midway_through_an_install_revokes_the_device_it_left_behind() {
    let bed = Bed::start().await;
    let bot = container_name("app-echo", "bot");
    let held = bed.docker.hold(&format!("POST /containers/{bot}/start"));
    let m = Arc::new(bed.manager());
    let task = tokio::spawn({
        let m = m.clone();
        async move { m.install(&echo(), &granted(), super::langs()).await }
    });
    held.reached().await;
    task.abort();
    task.await.unwrap_err();
    held.release();
    let orphan = bed.env();
    assert!(bed.token_valid(&orphan["TOKEN"]));

    bed.manager()
        .install(&echo(), &granted(), super::langs())
        .await
        .unwrap();
    let env = bed.env();
    assert!(!bed.token_valid(&orphan["TOKEN"]), "上次的令牌不能还能用");
    assert!(bed.token_valid(&env["TOKEN"]));
    assert_eq!(env["ROOM"], orphan["ROOM"], "账号和主房间沿用");
    assert_eq!(bed.tuwunel.state().devices("@echo:test").len(), 1);
}

#[tokio::test]
async fn an_install_failing_after_the_handover_takes_the_chat_back() {
    let bed = Bed::start().await;
    let m = bed.manager();
    // 安装记录写不进去（位置被一个目录占着）：这一步在把主房间交给主人之后。
    let record = bed.dir.path().join("plugins/app.echo.json");
    std::fs::create_dir_all(&record).unwrap();
    m.install(&echo(), &granted(), super::langs())
        .await
        .unwrap_err();
    let failed = bed.env();
    assert!(
        bed.tuwunel.state().rooms[&failed["ROOM"]]
            .members
            .is_empty(),
        "主人和 Bot 都退出了交出去的房间"
    );

    std::fs::remove_dir(&record).unwrap();
    m.install(&echo(), &granted(), super::langs())
        .await
        .unwrap();
    let room = bed.env()["ROOM"].clone();
    assert_ne!(room, failed["ROOM"]);
    assert!(
        bed.tuwunel.state().rooms[&room]
            .members
            .contains("@owner:test")
    );
}

#[tokio::test]
async fn a_restart_reports_a_bot_whose_device_was_revoked() {
    let bed = Bed::start().await;
    bed.manager()
        .install(&echo(), &granted(), super::langs())
        .await
        .unwrap();
    let restarted = bed.manager();
    restarted.restore().await.unwrap();
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);

    // 设备被吊销了（比如主人在别的客户端里删了它）：插件拿着的令牌已经失效，如实标为异常。
    let token = bed.env()["TOKEN"].clone();
    bed.tuwunel.state().tokens.remove(&token);
    let restarted = bed.manager();
    restarted.restore().await.unwrap();
    let PluginStatus::Degraded { reason } = &restarted.list()[0].status else {
        panic!("设备被吊销的 Bot 插件应标为异常");
    };
    assert!(
        reason.contains(&super::message(
            "plugin-settings-bot-device-revoked",
            &[("user", "@echo:test")],
        )),
        "{reason}"
    );
    // 状态异常也照样认得出来源：插件恢复正常前也能经核心连 Matrix。
    assert!(restarted.source(bed.source_address()).is_some());
}

#[tokio::test]
async fn bot_plugins_need_the_core_to_reach_matrix() {
    let bed = Bed::start().await;
    let m = manager_in_container(bed.engine.clone(), &bed.dir);
    let err = m
        .install(&echo(), &granted(), super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "matrix_unavailable", "{err}");
    assert!(bed.docker.networks().is_empty(), "什么都还没建");
}

#[tokio::test]
async fn the_echo_bot_example_gets_only_its_matrix_identity() {
    let package = example("echo-bot");
    let plan = dry_run(&package, &super::langs()).unwrap();
    let env = env_of(&plan.containers[0]);
    for var in ["HOMESERVER", "USER_ID", "ACCESS_TOKEN", "OWNER"] {
        let set = env
            .iter()
            .any(|e| e.starts_with(&format!("WONTON_MATRIX_{var}=")));
        assert!(set, "缺少 WONTON_MATRIX_{var}：{env:?}");
    }
    // 不出网、没有网页：只经核心的别名访问 Matrix。
    assert_eq!(package.manifest.permissions, granted());
    assert!(package.manifest.webui.is_none());

    let bed = Bed::start().await;
    let m = bed.manager();
    m.install_package(package, &granted(), super::langs())
        .await
        .unwrap();
    let bot = container_name("agent-echo", "bot");
    let user = bed.docker.env(&bot);
    assert!(
        user.contains(&"WONTON_MATRIX_USER_ID=@echo:test".to_owned()),
        "{user:?}"
    );
}

#[tokio::test]
async fn network_mode_bot_group_upgrade_preserves_its_identity_and_removes_the_whole_group() {
    let bed = Bed::start().await;
    let m = bed.manager();
    let source = echo()
        .replace(
            "x-wonton:",
            "  proxy:\n    image: busybox\n    network_mode: service:bot\nx-wonton:",
        )
        .replace(
            "  permissions: [matrix.bot]",
            "  bots: {services: [bot, proxy]}\n  permissions: [matrix.bot]",
        );
    m.install(&source, &granted(), super::langs())
        .await
        .unwrap();
    let before = bed.env();
    let package = super::super::PluginPackage::parse(
        &source.replace("version: 1.0.0", "version: 1.1.0"),
        &super::langs(),
    )
    .unwrap();
    m.upgrade_checked(package, &granted(), &[], false)
        .await
        .unwrap();
    assert_eq!(bed.env(), before);
    assert!(bed.token_valid(&before["TOKEN"]));
    let target = container_name("app-echo", "bot");
    let proxy = container_name("app-echo", "proxy");
    assert_eq!(
        bed.docker.created(&proxy).unwrap()["HostConfig"]["NetworkMode"],
        format!("container:{target}")
    );
    m.uninstall(&id(), false, &super::langs()).await.unwrap();
    assert!(!bed.docker.containers().contains(&target));
    assert!(!bed.docker.containers().contains(&proxy));
    assert!(!bed.token_valid(&before["TOKEN"]));
}
