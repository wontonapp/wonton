//! 多 Bot：加第二个 Bot、按 Bot 收密钥、删 Bot、共享 Bot，以及失败回滚留下的数据。

use serde_json::json;
use wonton_protocol::{BotKeeper, NewSharedBot};

use super::super::env_of_container;
use super::super::manager::manager_in_container;
use super::{Bed, agent, bot_container, echo, granted, granted_with_tools, id};
use crate::plugin::fake_docker::FakeFile;
use crate::plugin::install_error::InstallError;
use crate::plugin::store::Store;

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
    m.install(&agent(), &granted_with_tools(), super::super::langs())
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
    m.install(&compose, &granted_with_tools(), super::super::langs())
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
    m.install(&agent(), &granted_with_tools(), super::super::langs())
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
    m.install(&agent(), &granted_with_tools(), super::super::langs())
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
    m.uninstall(&id(), false, &super::super::langs())
        .await
        .unwrap();
    let single = m
        .create_shared_bot(&NewSharedBot {
            plugin_id: "app.echo".to_owned(),
            bot: "family".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(single, InstallError::NotInstalled(_)), "{single}");
    m.install(&echo(), &granted(), super::super::langs())
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
    m.install(&agent(), &granted_with_tools(), super::super::langs())
        .await
        .unwrap_err();
    let first = env_of_container(&bed.docker, &bot);
    let data = bot_data_dir(&bed, "default");
    assert!(data.is_dir(), "回滚留下这次建的 Bot 数据目录");
    // 插件的 init 写下的加密私钥。
    std::fs::write(data.join("crypto.key"), "private").unwrap();

    bed.docker
        .set_state(&bot, json!({"Status": "running", "Running": true}));
    m.install(&agent(), &granted_with_tools(), super::super::langs())
        .await
        .unwrap();
    let second = env_of_container(&bed.docker, &bot);
    assert_eq!(second["USER"], first["USER"]);
    assert_eq!(
        std::fs::read_to_string(data.join("crypto.key")).unwrap(),
        "private"
    );
}
