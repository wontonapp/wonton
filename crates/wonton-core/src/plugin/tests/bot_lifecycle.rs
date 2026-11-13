//! Bot 插件在 Matrix 上的副作用：在假 Docker 和假 Tuwunel 上验证安装、卸载、删除数据、回滚和核心重启。

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;

use super::super::docker::Engine;
use super::super::fake_docker::{FakeDocker, FakeFile};
use super::super::install_error::InstallError;
use super::super::names::container_name;
use super::super::package_files::PackageFiles;
use super::super::store::Store;
use super::super::{PluginId, PluginManager, PluginStatus};
use super::manager::manager_in_container;
use super::{env_of, env_of_container, example, package};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{Fake, start};
use crate::plugin::render::dry_run;
use wonton_protocol::{BotKeeper, NewSharedBot};

mod updates;

/// 一个回声 Bot：唯一的服务拿到 Bot 的全部 Matrix 变量。
fn echo() -> String {
    package(
        "  bot:\n    image: echo\n    environment:\n      \
         HS: ${WONTON_MATRIX_HOMESERVER}\n      USER: ${WONTON_MATRIX_USER_ID}\n      \
         TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}\n      ALLOWED: ${WONTON_MATRIX_ALLOWED_USERS}\n      \
         ROOM: ${WONTON_MATRIX_HOME_ROOM}",
        "  bot: {display_name: {zh: 回声, en: Echo}}\n  permissions: [matrix.bot]",
    )
    .replace("id: app.test", "id: app.echo")
}

fn granted() -> Vec<super::super::Permission> {
    vec!["matrix.bot".parse().unwrap()]
}

/// 服务容器名：Bot 服务的容器带 `--<服务>--<Bot>` 后缀。
fn service_container(slug: &str, service: &str) -> String {
    super::super::names::Instance::default().bot_container(slug, service, "default")
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
        // 声明了 `bots` 的插件按 Bot 一份容器（带后缀），单 Bot 的插件还是普通服务名。
        let suffixed = service_container("app-echo", "bot");
        let env = if self.docker.created(&suffixed).is_some() {
            self.docker.env(&suffixed)
        } else {
            self.docker.env(&container_name("app-echo", "bot"))
        };
        env.iter()
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
    assert_eq!(env["ALLOWED"], "@owner:test");
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
    let bot = report.bots.first().unwrap().clone();
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
    assert!(report.bots.first().unwrap().deactivated);
    assert!(bed.tuwunel.state().user("@echo:test").deactivated);
    let store = Store::open(bed.dir.path().join("plugins")).unwrap();
    assert!(store.bots().load(&id()).unwrap().bots.is_empty());
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
    // 安装要是先失败就没人在等这个请求；超时把安装任务的状态和错误一并带出来，别只剩一句超时。
    if tokio::time::timeout(std::time::Duration::from_secs(5), held.reached())
        .await
        .is_err()
    {
        let install = if task.is_finished() {
            match task.await {
                Ok(Ok(_)) => "安装已经完成".to_owned(),
                Ok(Err(e)) => format!("安装先失败了：{e}"),
                Err(e) => format!("安装任务结束了：{e}"),
            }
        } else {
            "安装还在跑".to_owned()
        };
        panic!("5 秒内没有等到假 Docker 收到 POST /containers/{bot}/start，{install}");
    }
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
    for var in ["HOMESERVER", "USER_ID", "ACCESS_TOKEN", "ALLOWED_USERS"] {
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
    // 这个示例还没声明 `bots`：单个 Bot 的插件，容器名不带 Bot 后缀。
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
    // bot 和 proxy 都是 Bot 服务：容器按 Bot 一份，proxy 共用同一个 Bot 那份 bot 容器的网络命名空间。
    let target = service_container("app-echo", "bot");
    let proxy = service_container("app-echo", "proxy");
    let env_map = |container: &str| {
        bed.docker
            .env(container)
            .iter()
            .filter_map(|e| e.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect::<std::collections::BTreeMap<String, String>>()
    };
    let before = env_map(&target);
    let package = super::super::PluginPackage::parse(
        &source.replace("version: 1.0.0", "version: 1.1.0"),
        &super::langs(),
    )
    .unwrap();
    m.upgrade_checked(package, &granted(), &[], false, Default::default())
        .await
        .unwrap();
    assert_eq!(env_map(&target), before);
    assert!(bed.token_valid(&before["TOKEN"]));
    assert_eq!(
        bed.docker.created(&proxy).unwrap()["HostConfig"]["NetworkMode"],
        format!("container:{target}")
    );
    m.uninstall(&id(), false, &super::langs()).await.unwrap();
    assert!(!bed.docker.containers().contains(&target));
    assert!(!bed.docker.containers().contains(&proxy));
    assert!(!bed.token_valid(&before["TOKEN"]));
}

/// 多 Bot 的 Agent：`bot` 服务每个 Bot 一份，挂自己的数据目录，拿到 Bot 名、总线地址和 Matrix 身份。
fn agent() -> String {
    package(
        "  bot:\n    image: echo\n    volumes: ['${WONTON_BOT_DATA}:/data']\n    environment:\n      \
         HS: ${WONTON_MATRIX_HOMESERVER}\n      USER: ${WONTON_MATRIX_USER_ID}\n      \
         TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}\n      ALLOWED: ${WONTON_MATRIX_ALLOWED_USERS}\n      \
         ROOM: ${WONTON_MATRIX_HOME_ROOM}\n      BOT: ${WONTON_BOT_NAME}\n      MCP: ${WONTON_MCP_URL}",
        "  bot: {display_name: {zh: 回声, en: Echo}}\n  agent: {protocol: matrix}\n  \
         bots: {services: [bot]}\n  permissions: [matrix.bot, tools]",
    )
    .replace("id: app.test", "id: app.echo")
    .replace("kind: app", "kind: agent")
}

fn granted_with_tools() -> Vec<super::super::Permission> {
    granted()
        .into_iter()
        .chain(["tools".parse().unwrap()])
        .collect()
}

fn bot_container(bot: &str) -> String {
    use super::super::names::Instance;
    Instance::default().bot_container("app-echo", "bot", bot)
}

fn bot_data_dir(bed: &Bed, bot: &str) -> std::path::PathBuf {
    // 应用数据目录按插件 ID 的 slug（app.echo → app-echo）。
    bed.dir
        .path()
        .join("apps")
        .join("app-echo")
        .join("bots")
        .join(bot)
}

#[tokio::test]
async fn a_second_bot_gets_its_own_identity_and_the_first_is_not_rebuilt() {
    let bed = Bed::start().await;
    let m = bed.manager();
    m.install(&agent(), &granted_with_tools(), super::langs())
        .await
        .unwrap();
    let default_env = env_of_container(&bed.docker, &bot_container("default"));
    assert_eq!(default_env["BOT"], "default");
    assert_eq!(default_env["ALLOWED"], "@owner:test");
    assert!(default_env["MCP"].contains("/mcp/"));
    assert_eq!(bed.docker.creations(&bot_container("default")), 1);

    let created = m
        .add_bot(
            &id(),
            "alice",
            BotKeeper::User {
                username: "alice".to_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(created.bot, "alice");
    assert_eq!(created.user_id, "@echo-alice:test");

    // 每组容器自己的 Bot 名、Matrix 身份、总线令牌。
    let alice_env = env_of_container(&bed.docker, &bot_container("alice"));
    assert_eq!(alice_env["BOT"], "alice");
    assert_eq!(alice_env["USER"], "@echo-alice:test");
    assert_eq!(alice_env["ALLOWED"], "@alice:test");
    assert_ne!(alice_env["TOKEN"], default_env["TOKEN"]);
    let default_url = default_env["MCP"].clone();
    let alice_url = alice_env["MCP"].clone();
    assert_ne!(default_url.rsplit('/').next(), alice_url.rsplit('/').next());
    assert_ne!(default_url, alice_url);
    assert!(bed.token_valid(&default_env["TOKEN"]));
    assert!(bed.token_valid(&alice_env["TOKEN"]));
    // 主房间交给了归属人。
    assert!(
        bed.tuwunel.state().rooms[&created.home_room]
            .members
            .contains("@alice:test")
    );
    // 记录里有归属人；第一个 Bot 的容器没有重建。
    let bots = Store::open(bed.dir.path().join("plugins"))
        .unwrap()
        .bots()
        .load(&id())
        .unwrap();
    assert_eq!(
        bots.bots["alice"].keeper,
        BotKeeper::User {
            username: "alice".to_owned()
        }
    );
    assert_eq!(
        bots.bots["default"].keeper,
        BotKeeper::User {
            username: "owner".to_owned()
        }
    );
    assert_eq!(bed.docker.creations(&bot_container("default")), 1);
}

#[tokio::test]
async fn collected_secrets_from_bot_services_are_kept_per_bot() {
    let bed = Bed::start().await;
    let m = bed.manager();
    let compose = agent().replace(
        "  permissions: [matrix.bot, tools]",
        "  permissions: [matrix.bot, tools]\n  secrets:\n    recovery:\n      \
         collect: {service: bot, path: /data/key.txt}",
    );
    m.install(&compose, &granted_with_tools(), super::langs())
        .await
        .unwrap();
    m.add_bot(
        &id(),
        "alice",
        BotKeeper::User {
            username: "alice".to_owned(),
        },
    )
    .await
    .unwrap();

    // 每个 Bot 容器写下自己的恢复密钥（内容不同）：核心按 Bot 各收一份。
    let file = |text: &str| Some(FakeFile::Regular(text.as_bytes().to_vec()));
    bed.docker.set_file(
        &bot_container("default"),
        "/data/key.txt",
        file("key-default"),
    );
    bed.docker
        .set_file(&bot_container("alice"), "/data/key.txt", file("key-alice"));
    manager_in_container(bed.engine.clone(), &bed.dir)
        .with_matrix(Some(bed.matrix.clone()))
        .restore()
        .await
        .unwrap();
    let stored = || {
        Store::open(bed.dir.path().join("plugins"))
            .unwrap()
            .secrets()
            .load(&id())
            .unwrap()
            .unwrap_or_default()
    };
    assert_eq!(stored()["recovery@default"], b"key-default");
    assert_eq!(stored()["recovery@alice"], b"key-alice");

    // 删 Bot 时它那份收存的密钥清掉，别的 Bot 的不动。
    m.remove_bot(&id(), "alice", true).await.unwrap();
    let stored = stored();
    assert!(!stored.contains_key("recovery@alice"));
    assert_eq!(stored["recovery@default"], b"key-default");
}

#[tokio::test]
async fn removing_a_bot_only_touches_its_containers_and_honors_keep_data() {
    let bed = Bed::start().await;
    let m = bed.manager();
    m.install(&agent(), &granted_with_tools(), super::langs())
        .await
        .unwrap();
    m.add_bot(
        &id(),
        "alice",
        BotKeeper::User {
            username: "alice".to_owned(),
        },
    )
    .await
    .unwrap();
    assert!(bot_data_dir(&bed, "alice").is_dir());
    let default_token = env_of_container(&bed.docker, &bot_container("default"))["TOKEN"].clone();

    // 不留数据：容器删掉、账号停用（不可恢复）、记录删掉、数据目录删掉。
    let report = m.remove_bot(&id(), "alice", false).await.unwrap();
    assert_eq!(report.user_id, "@echo-alice:test");
    assert!(report.deactivated);
    assert!(bed.tuwunel.state().user("@echo-alice:test").deactivated);
    assert!(!bed.docker.containers().contains(&bot_container("alice")));
    assert!(!bot_data_dir(&bed, "alice").exists());
    let bots = Store::open(bed.dir.path().join("plugins"))
        .unwrap()
        .bots()
        .load(&id())
        .unwrap();
    assert!(!bots.bots.contains_key("alice"));
    assert!(bots.bots.contains_key("default"));

    // 留数据：容器删掉、账号还在、数据目录还在。
    m.add_bot(
        &id(),
        "bob",
        BotKeeper::User {
            username: "bob".to_owned(),
        },
    )
    .await
    .unwrap();
    let bob_token = env_of_container(&bed.docker, &bot_container("bob"))["TOKEN"].clone();
    let report = m.remove_bot(&id(), "bob", true).await.unwrap();
    assert!(!report.deactivated);
    assert!(!bed.tuwunel.state().user("@echo-bob:test").deactivated);
    // 账号留着，但设备照样吊销：Bot 没了不能再留一把还能用的令牌。
    assert!(!bed.token_valid(&bob_token));
    assert!(bot_data_dir(&bed, "bob").exists());
    // 默认 Bot 的容器没动过，令牌还有效。
    assert_eq!(bed.docker.creations(&bot_container("default")), 1);
    assert!(bed.token_valid(&default_token));
}

#[tokio::test]
async fn a_shared_bot_belongs_to_the_household_and_validations_hold() {
    let bed = Bed::start().await;
    let m = bed.manager();
    m.install(&agent(), &granted_with_tools(), super::langs())
        .await
        .unwrap();

    let created = m
        .create_shared_bot(&NewSharedBot {
            plugin_id: "app.echo".to_owned(),
            bot: "family".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(
        (created.plugin_id.as_str(), created.bot.as_str()),
        ("app.echo", "family")
    );
    let env = env_of_container(&bed.docker, &bot_container("family"));
    assert_eq!(env["USER"], "@echo-family:test");
    // 共享 Bot 归全家：名单按主人算，主房间交给主人。
    assert_eq!(env["ALLOWED"], "@owner:test");
    let bots = Store::open(bed.dir.path().join("plugins"))
        .unwrap()
        .bots()
        .load(&id())
        .unwrap();
    assert_eq!(bots.bots["family"].keeper, BotKeeper::Household);
    {
        // 锁守卫不能横跨后面的 await：假 Tuwunel 的处理器在同一把锁上等，会卡死运行时。
        let state = bed.tuwunel.state();
        assert!(state.rooms[&env["ROOM"]].members.contains("@owner:test"));
    }

    // 重名：409。
    let again = m
        .create_shared_bot(&NewSharedBot {
            plugin_id: "app.echo".to_owned(),
            bot: "family".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(again, InstallError::Conflict(_)), "{again}");

    // 名字不合规则：400。
    let bad = m
        .create_shared_bot(&NewSharedBot {
            plugin_id: "app.echo".to_owned(),
            bot: "Family!".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(bad, InstallError::Invalid(_)), "{bad}");

    // 没声明 bots 的插件：明确的错误，不退回成单 Bot。
    m.uninstall(&id(), false, &super::langs()).await.unwrap();
    let single = m
        .create_shared_bot(&NewSharedBot {
            plugin_id: "app.echo".to_owned(),
            bot: "family".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(single, InstallError::NotInstalled(_)), "{single}");
    m.install(&echo(), &granted(), super::langs())
        .await
        .unwrap();
    let single = m
        .create_shared_bot(&NewSharedBot {
            plugin_id: "app.echo".to_owned(),
            bot: "family".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(
        matches!(single, InstallError::Invalid(ref e) if e.contains("不支持多个 Bot")),
        "{single}"
    );
}

/// 安装失败回滚、账号留给下次时，这次建的 Bot 数据也留下：init 在服务器上建的加密身份，私钥只在这里，删了
/// 下次就解不开（Tuwunel 删不掉也换不掉那个身份）。重试沿用同一个账号和同一份数据。
#[tokio::test]
async fn a_failed_install_keeps_the_bot_data_for_the_same_account_next_time() {
    let bed = Bed::start().await;
    let m = bed.manager();
    let bot = bot_container("default");
    let failed = json!({"Status": "exited", "Running": false, "ExitCode": 1});
    bed.docker.set_state(&bot, failed);
    m.install(&agent(), &granted_with_tools(), super::langs())
        .await
        .unwrap_err();
    let first = env_of_container(&bed.docker, &bot);
    let data = bot_data_dir(&bed, "default");
    assert!(data.is_dir(), "回滚留下这次建的 Bot 数据目录");
    // 插件的 init 写下的加密私钥。
    std::fs::write(data.join("crypto.key"), "private").unwrap();

    bed.docker
        .set_state(&bot, json!({"Status": "running", "Running": true}));
    m.install(&agent(), &granted_with_tools(), super::langs())
        .await
        .unwrap();
    let second = env_of_container(&bed.docker, &bot);
    assert_eq!(second["USER"], first["USER"]);
    assert_eq!(
        std::fs::read_to_string(data.join("crypto.key")).unwrap(),
        "private"
    );
}

const LEVEL_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "level": {
      "type": "string",
      "default": "quiet",
      "x-wonton-title": { "zh": "音量", "en": "Level" }
    }
  }
}"#;

fn echo_with_setting() -> String {
    echo()
        .replace("spec: '0.5'", "spec: '0.12'")
        .replace(
            "ROOM: ${WONTON_MATRIX_HOME_ROOM}",
            "ROOM: ${WONTON_MATRIX_HOME_ROOM}\n      LEVEL: ${WONTON_SETTING_LEVEL}",
        )
        .replace(
            "  permissions: [matrix.bot]",
            "  settings: settings.schema.json\n  permissions: [matrix.bot]",
        )
}

#[tokio::test]
async fn writing_settings_recreates_the_bot_with_its_identity_kept() {
    let bed = Bed::start().await;
    let m = Arc::new(bed.manager());
    let files: PackageFiles = [(
        "settings.schema.json".to_owned(),
        LEVEL_SCHEMA.as_bytes().to_vec(),
    )]
    .into();
    let pkg = super::super::PluginPackage::with_files(&echo_with_setting(), files, &super::langs())
        .unwrap();
    m.install_user_enabled(
        pkg,
        &granted(),
        &[],
        false,
        std::collections::BTreeMap::from([("level".to_owned(), json!("quiet"))]),
        super::langs(),
    )
    .await
    .unwrap();
    let before = bed.env();
    assert_eq!(before["LEVEL"], "quiet");

    let digest = m
        .plugin_settings(&id(), &super::langs())
        .await
        .unwrap()
        .schema_digest;
    let update = wonton_protocol::PluginSettingsUpdate {
        schema_digest: digest,
        values: std::collections::HashMap::from([(
            "level".to_owned(),
            Some(wonton_protocol::SettingValue::Text {
                value: "loud".to_owned(),
            }),
        )]),
    };
    let view = m
        .set_plugin_settings(&id(), &update, &super::langs())
        .await
        .unwrap();
    assert_eq!(view.form.fields[0].value, update.values["level"]);

    let after = bed.env();
    assert_eq!(after["LEVEL"], "loud");
    for key in ["HS", "USER", "TOKEN", "ALLOWED", "ROOM"] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert!(bed.token_valid(&after["TOKEN"]));
}
