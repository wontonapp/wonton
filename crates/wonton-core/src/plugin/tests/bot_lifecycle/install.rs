//! Bot 插件安装：交出的身份、失败回滚、没有核心 Matrix 时的报错和回声示例。

use std::collections::BTreeSet;

use serde_json::json;

use super::super::example;
use super::super::manager::manager_in_container;
use super::{Bed, echo, granted, id};
use crate::plugin::names::container_name;
use crate::plugin::render::dry_run;
use crate::plugin::tests::env_of;

#[tokio::test]
async fn a_bot_plugin_gets_its_own_matrix_identity_through_the_core() {
    let bed = Bed::start().await;
    let m = bed.manager();
    m.install(&echo(), &granted(), super::super::langs())
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
        assert!(!room.encrypted && room.members.contains("@owner:test"));
        assert!(
            !state.room_state[&env["ROOM"]]
                .events
                .contains_key(&("m.room.encryption".to_owned(), String::new()))
        );
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
async fn a_failed_install_leaves_the_owner_no_chat_and_keeps_the_home_room_for_next_time() {
    let bed = Bed::start().await;
    let m = bed.manager();
    let bot = container_name("app-echo", "bot");
    bed.docker.set_state(
        &bot,
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    m.install(&echo(), &granted(), super::super::langs())
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
            room.left.contains("@owner:test"),
            "主人建完就退出了，房间不在主人的聊天列表里"
        );
        assert!(room.invited.is_empty(), "邀请已经被 Bot 接受");
        assert_eq!(state.user("@owner:test").direct, None, "没交付，不算私聊");
    }

    bed.docker
        .set_state(&bot, json!({"Status": "running", "Running": true}));
    m.install(&echo(), &granted(), super::super::langs())
        .await
        .unwrap();
    assert_eq!(
        bed.env()["ROOM"],
        failed["ROOM"],
        "重试沿用上次建好的主房间"
    );
    let state = bed.tuwunel.state();
    assert_eq!(
        state
            .rooms
            .keys()
            .filter(|id| id.as_str() != crate::matrix::fake::ADMIN_ROOM_ID)
            .count(),
        1
    );
    assert!(state.rooms[&failed["ROOM"]].members.contains("@owner:test"));
}

#[tokio::test]
async fn an_install_failing_after_the_handover_takes_the_chat_back() {
    let bed = Bed::start().await;
    let m = bed.manager();
    // 安装记录写不进去（位置被一个目录占着）：这一步在把主房间交给主人之后。
    let record = bed.dir.path().join("plugins/app.echo.json");
    std::fs::create_dir_all(&record).unwrap();
    m.install(&echo(), &granted(), super::super::langs())
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
    m.install(&echo(), &granted(), super::super::langs())
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
async fn bot_plugins_need_the_core_to_reach_matrix() {
    let bed = Bed::start().await;
    let m = manager_in_container(bed.engine.clone(), &bed.dir);
    let err = m
        .install(&echo(), &granted(), super::super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "matrix_unavailable", "{err}");
    assert!(bed.docker.networks().is_empty(), "什么都还没建");
}

#[tokio::test]
async fn the_echo_bot_example_gets_only_its_matrix_identity() {
    let package = example("echo-bot");
    let plan = dry_run(&package, &super::super::langs()).unwrap();
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
    m.install_package(package, &granted(), super::super::langs())
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
