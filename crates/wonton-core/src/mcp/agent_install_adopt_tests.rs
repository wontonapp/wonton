//! 插件归属的更新和接手（ADR-0033、#817）：同一个 Bot 装两次走更新，别的 Bot 照旧拒绝；
//! 主人自己装的插件，Agent 经卡片接手。

use std::collections::BTreeMap;
use std::fs;

use serde_json::json;
use wonton_protocol::{Answer, BotKeeper};

use super::agent_install_tests::{INSTALL, bench, plain};
use super::tests::{failed, text};
use crate::mcp::Caller;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage};

/// Bot 调用方：令牌认出的 Bot，归属按插件 ID 记。
fn bot_caller(id: &str) -> Caller {
    let title = [("zh".to_owned(), "测试 Bot".to_owned())].into();
    Caller::for_bot(
        PluginId::parse(id).unwrap(),
        Some(&title),
        "default".to_owned(),
        BotKeeper::User {
            username: "owner".to_owned(),
        },
    )
}

/// 主人经 App 上传装的包（没有归属、没有来源）。
fn uploaded_package(compose: &str, index: &str) -> PluginPackage {
    let mut files = BTreeMap::new();
    files.insert("index.html".to_owned(), index.as_bytes().to_vec());
    PluginPackage::with_files(compose, files, &Languages::from_header(Some("zh-Hans"))).unwrap()
}

#[tokio::test]
async fn the_same_bot_updates_its_own_plugin() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);

    // 第一次：装。批准后归属是这个 Bot 的插件。
    let bot = bot_caller("com.example.agent");
    let call = b.spawn_as(&bot, INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(1).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert_eq!(
        b.writer("app.agent.hello").unwrap().plugin_id,
        "com.example.agent"
    );

    // 第二次：同一个 Bot 改了内容再调，走更新而不是报错。
    b.write("hello", "index.html", "<h1>二</h1>");
    let result = b
        .spawn_as(&bot, INSTALL, json!({"dir": "hello"}))
        .await
        .unwrap();
    assert!(!failed(&result), "{result:?}");
    let record =
        b.s.bus
            .plugins
            .installed_record(&PluginId::parse("app.agent.hello").unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(
        record.files.get("index.html").unwrap(),
        "<h1>二</h1>".as_bytes()
    );
    assert_eq!(
        b.writer("app.agent.hello").unwrap().plugin_id,
        "com.example.agent",
        "更新后归属还在"
    );

    // 另一个 Bot（别的插件，自己的令牌和 outbox）拿着同一个插件包来更新：照旧拒绝，零卡片。
    let other = bot_caller("com.example.other");
    let other_outbox = b.s.bus.plugins.app_data_dir(&other.id).join("outbox");
    fs::create_dir_all(other_outbox.join("hello")).unwrap();
    fs::write(
        other_outbox.join("hello/docker-compose.yml"),
        format!("name: app\n{services}\nx-wonton:\n{x_wonton}"),
    )
    .unwrap();
    fs::write(other_outbox.join("hello/index.html"), "<h1>三</h1>").unwrap();
    let before = b.s.cards.posted.lock().unwrap().len();
    let result = b
        .spawn_as(&other, INSTALL, json!({"dir": "hello"}))
        .await
        .unwrap();
    assert!(
        failed(&result) && text(&result).contains("不是你写的插件"),
        "{result:?}"
    );
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before, "零卡片");
}

/// 主人自己上传的插件（App 装，没有归属）：Agent 拿同一个包来更新，发卡片请主人点头，
/// 批准后更新并记下归属；以后同一份摘要再交就是“没有变化”。
#[tokio::test]
async fn an_agent_adopts_an_owner_installed_plugin_through_the_card() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    let compose = format!("name: app\n{services}\nx-wonton:\n{x_wonton}");
    let package = uploaded_package(&compose, "<h1>主人装的</h1>");
    let granted = package.manifest.permissions.clone();
    b.s.manager
        .install_detached_enabled(
            package,
            granted,
            Vec::new(),
            false,
            Languages::from_header(Some("zh")),
        )
        .await
        .unwrap();
    assert_eq!(b.writer("app.agent.hello"), None);

    // 主人用过的数据留在原地：接手不许动数据。
    let data =
        b.s.bus
            .plugins
            .app_data_dir(&PluginId::parse("app.agent.hello").unwrap());
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("keep.txt"), "在的").unwrap();

    // Agent 改了内容再来：发卡片（接手不能安静更新），批准后归属是这个 Bot。
    b.write_package("hello", services, &x_wonton);
    b.write("hello", "index.html", "<h1>Agent 改的</h1>");
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(1).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert_eq!(
        b.writer("app.agent.hello").unwrap().plugin_id,
        "com.example.agent"
    );
    let record =
        b.s.bus
            .plugins
            .installed_record(&PluginId::parse("app.agent.hello").unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(
        record.files.get("index.html").unwrap(),
        "<h1>Agent 改的</h1>".as_bytes()
    );
    assert_eq!(
        fs::read_to_string(data.join("keep.txt")).unwrap(),
        "在的",
        "接手后数据还在"
    );

    // 归属记下后，同一份摘要再交：报没有变化，不再发卡片。
    let before = b.s.cards.posted.lock().unwrap().len();
    let result = b.s.call(INSTALL, json!({"dir": "hello"})).await;
    assert!(
        failed(&result) && text(&result).contains("没有变化"),
        "{result:?}"
    );
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before);
}

/// 主人回绝接手后：Bot 拿到明确的拒绝，冷却期内同一个 Bot 对同一个插件不再发卡、零卡片。
#[tokio::test]
async fn a_refused_adoption_cools_down() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    let compose = format!("name: app\n{services}\nx-wonton:\n{x_wonton}");
    let package = uploaded_package(&compose, "<h1>主人装的</h1>");
    let granted = package.manifest.permissions.clone();
    b.s.manager
        .install_detached_enabled(
            package,
            granted,
            Vec::new(),
            false,
            Languages::from_header(Some("zh")),
        )
        .await
        .unwrap();

    // 第一次：发卡片，主人拒绝。
    b.write_package("hello", services, &x_wonton);
    b.write("hello", "index.html", "<h1>Agent 改的</h1>");
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(1).await;
    b.s.answer(&card.id, Answer::Reject).unwrap();
    let result = call.await.unwrap();
    assert!(failed(&result), "{result:?}");
    assert_eq!(b.writer("app.agent.hello"), None, "拒绝后归属不变");

    // 冷却期内再来：不再发卡，直接说明刚才被回绝。
    let before = b.s.cards.posted.lock().unwrap().len();
    let result = b.s.call(INSTALL, json!({"dir": "hello"})).await;
    assert!(
        failed(&result) && text(&result).contains("回绝"),
        "{result:?}"
    );
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before, "零卡片");
}

/// 主人经文件升级后，归属清掉：Agent 再更新不再直接拒绝，而是发卡片请主人点头，
/// 批准后更新并记回归属。
#[tokio::test]
async fn an_owner_upgrade_takes_the_plugin_back_from_the_agent() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    b.installed_with_answer("hello", 1, Answer::Approve).await;
    let id = PluginId::parse("app.agent.hello").unwrap();
    assert!(b.writer("app.agent.hello").is_some());

    // App 的升级路径升级成功后清归属（和网关 handler 一样调 set_writer）。
    let (services, x_wonton) = plain("app.agent.hello", "0.2.0", "");
    let compose = format!("name: app\n{services}\nx-wonton:\n{x_wonton}");
    let package = uploaded_package(&compose, "<h1>主人升的</h1>");
    let granted = package.manifest.permissions.clone();
    b.s.manager
        .upgrade_upload(
            &id,
            package,
            granted,
            Vec::new(),
            false,
            &Languages::from_header(Some("zh")),
        )
        .await
        .unwrap();
    b.s.manager.set_writer(&id, None).unwrap();

    // Agent 拿 0.3.0 来更新：发卡片（接手不能安静更新），批准后归属是这个 Bot。
    let (services, x_wonton) = plain("app.agent.hello", "0.3.0", "");
    b.write_package("hello", services, &x_wonton);
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(2).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert_eq!(
        b.writer("app.agent.hello").unwrap().plugin_id,
        "com.example.agent"
    );
    let record = b.s.bus.plugins.installed_record(&id).unwrap().unwrap();
    assert_eq!(record.version, "0.3.0");
}
