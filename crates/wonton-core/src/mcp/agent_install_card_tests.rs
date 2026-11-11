//! 审查 #374 补的行为测试：id 路径的两张卡、第二张被取代、不打扰的更新也让旧卡作废、
//! 「不再问」只在没有很高或最高级权限时写、以前留下的数据要醒目提示、减权限按新清单授予。
//! 共用的夹具在 [`super::agent_install_tests`]。

use std::fs;

use serde_json::json;
use wonton_protocol::Answer;

use super::agent_install_tests::{INSTALL, bench, plain, plain_root};
use super::tests::{failed, text};
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;

#[tokio::test]
async fn an_official_highest_permission_install_needs_both_cards() {
    let b = bench().await;
    // 官方索引里的最高级插件：docker.socket 权限要配上对应的挂载（ADR-0036）。
    let source = official_with_mount("app.example.root", "全能", "docker.socket");
    let package =
        crate::plugin::PluginPackage::parse(&source, &Languages::from_header(Some("zh-Hans")))
            .unwrap();
    let entry = crate::plugin::storefront::index::IndexEntry::of(
        &package,
        "github:example/root",
        &"a".repeat(40),
    )
    .unwrap();
    entry
        .write_to(&b.s.dir.path().join("index"), &package)
        .unwrap();

    let call = b.s.spawn(INSTALL, json!({"id": "app.example.root"}));
    let card = b.s.cards.wait_for(1).await;
    assert!(card.title.contains("全能"), "{}", card.title);
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let card = b.s.cards.wait_for(2).await;
    assert!(card.title.contains("能控制整台主机"), "{}", card.title);
    assert!(!b.installed("app.example.root"), "第二张没批之前不装");
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.installed("app.example.root"));
}

/// 官方索引测试包的变体：带一个 Docker socket 挂载，配 docker.socket 权限。
fn official_with_mount(id: &str, title: &str, permission: &str) -> String {
    format!(
        "name: app
services:
  web:
    image: nginx
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
x-wonton:
  spec: '0.4'
  id: {id}
  version: 1.0.0
  kind: app
  title: {{zh: {title}, en: {title}}}
  description: {{zh: {title}的说明, en: About {title}}}
  permissions: [{permission}]
"
    )
}

/// 等第二张“我知道”时又交了一份新的：旧调用被取代（什么都没装），新的重新走两张卡。
#[tokio::test]
async fn a_superseded_second_card_installs_nothing() {
    let b = bench().await;
    let (services, x_wonton) = plain_root("app.agent.root", "0.1.0");
    b.write_package("root", services, &x_wonton);
    let first = b.s.spawn(INSTALL, json!({"dir": "root"}));
    let card = b.s.cards.wait_for(1).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    b.s.cards.wait_for(2).await;

    // 新提交让还在等的第二张卡作废。
    let (services, x_wonton) = plain_root("app.agent.root", "0.2.0");
    b.write_package("root", services, &x_wonton);
    let second = b.s.spawn(INSTALL, json!({"dir": "root"}));
    let result = first.await.unwrap();
    assert!(
        failed(&result) && text(&result).contains("取代"),
        "{result:?}"
    );
    assert!(!b.installed("app.agent.root"));

    // 新提交重新走两张卡，批完装上。
    let card = b.s.cards.wait_for(3).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let card = b.s.cards.wait_for(4).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = second.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.installed("app.agent.root"));
}

/// 不打扰主人的更新也会让还在等的卡片作废：不能让主人批一份已经被盖掉的。
#[tokio::test]
async fn a_quiet_update_supersedes_the_pending_card() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    b.installed_with_answer("hello", 1, Answer::Approve).await;

    // 变宽的更新在等主人。
    let (services, x_wonton) = plain("app.agent.hello", "0.2.0", "storage.share.documents:read");
    b.write_package("hello", services, &x_wonton);
    let waiting = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(2).await;

    // 不变宽的更新直接装上，等着的卡片作废。
    let (services, x_wonton) = plain("app.agent.hello", "0.2.1", "");
    b.write_package("hello", services, &x_wonton);
    let result = b.s.call(INSTALL, json!({"dir": "hello"})).await;
    assert!(!failed(&result), "{result:?}");
    assert!(text(&result).contains("0.2.1"));
    let waiting = waiting.await.unwrap();
    assert!(
        failed(&waiting) && text(&waiting).contains("取代"),
        "{waiting:?}"
    );
    assert!(
        b.s.answer(&card.id, Answer::Approve).is_err(),
        "旧卡片已作废"
    );
    let id = PluginId::parse("app.agent.hello").unwrap();
    let record = b.s.bus.plugins.installed_record(&id).unwrap().unwrap();
    assert_eq!(record.version, "0.2.1");
}

/// 普通插件的首装卡片说“以后不再问”；有很高或最高级权限的没这句（这些每次都问）。
#[tokio::test]
async fn the_never_ask_line_matches_the_risk() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(1).await;
    assert!(
        card.lines.iter().any(|l| l.text.contains("不再问")),
        "{card:?}"
    );
    b.s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();

    let (services, x_wonton) = plain_root("app.agent.root", "0.1.0");
    b.write_package("root", services, &x_wonton);
    let call = b.s.spawn(INSTALL, json!({"dir": "root"}));
    let card = b.s.cards.wait_for(2).await;
    assert!(
        !card.lines.iter().any(|l| l.text.contains("不再问")),
        "{card:?}"
    );
    b.s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();
}

/// 以前卸载没删数据和密钥：dir 和 id 的首装卡片都醒目提示，装上就能读到。
#[tokio::test]
async fn retained_data_is_called_out_on_first_install_cards() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    // 以前的数据目录还在（不删数据的卸载留下的）。
    let leftover = b.s.dir.path().join("apps").join("app-agent-hello");
    fs::create_dir_all(&leftover).unwrap();
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(1).await;
    let expected = crate::l10n::text(
        &b.s.bus.plugins.owner_languages(),
        "install-retained-data",
        &[("id", "app.agent.hello".into())],
    );
    assert!(
        card.lines.iter().any(|l| l.text == expected && l.emphasis),
        "{card:?}"
    );
    b.s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();

    // id 路径一样：官方索引里的 journal，留着同 ID 的旧目录。
    let leftover = b.s.dir.path().join("apps").join("app-example-journal");
    fs::create_dir_all(&leftover).unwrap();
    let call = b.s.spawn(INSTALL, json!({"id": "app.example.journal"}));
    let card = b.s.cards.wait_for(2).await;
    let expected = crate::l10n::text(
        &b.s.bus.plugins.owner_languages(),
        "install-retained-data",
        &[("id", "app.example.journal".into())],
    );
    assert!(
        card.lines.iter().any(|l| l.text == expected && l.emphasis),
        "{card:?}"
    );
    b.s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();
}

/// 更新把权限变少：授予的跟着新清单走，旧授予的多余项不保留。
#[tokio::test]
async fn a_shrinking_update_grants_only_the_new_manifest() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "storage.share.documents:read");
    b.write_package("hello", services, &x_wonton);
    b.installed_with_answer("hello", 1, Answer::Approve).await;
    let id = PluginId::parse("app.agent.hello").unwrap();
    let record = b.s.bus.plugins.installed_record(&id).unwrap().unwrap();
    assert_eq!(record.granted_permissions.len(), 1);

    // 减权限不是变宽：不打扰主人，直接更新，授予变成空。
    let (services, x_wonton) = plain("app.agent.hello", "0.2.0", "");
    b.write_package("hello", services, &x_wonton);
    let result = b.s.call(INSTALL, json!({"dir": "hello"})).await;
    assert!(!failed(&result), "{result:?}");
    let record = b.s.bus.plugins.installed_record(&id).unwrap().unwrap();
    assert_eq!(record.version, "0.2.0");
    assert!(
        record.granted_permissions.is_empty(),
        "{:?}",
        record.granted_permissions
    );
}

/// Agent 写的插件带一项主人填写的密钥：卡片列出它，Bot 传的任何 `secrets` 参数都不算值，
/// 主人回答时缺必填项卡片不结束。
#[tokio::test]
async fn the_card_asks_for_input_secrets_and_the_bot_cannot_fill_them() {
    let b = bench().await;
    let (services, mut x_wonton) = plain("app.input", "1.0.0", "");
    x_wonton = format!(
        "{x_wonton}  secrets:\n    token:\n      input:\n        title: {{zh: 令牌, en: Token}}\n        required: true\n"
    );
    let services = services.replace(
        "    configs:",
        "    environment:\n      TOKEN: ${WONTON_SECRET_TOKEN}\n    configs:",
    );
    b.write_package("input", &services, &x_wonton);
    // Bot 自己带了一个 secrets 参数：工具不收它，卡片照常要填。
    let call = b.s.spawn(
        INSTALL,
        json!({"dir": "input", "secrets": {"token": "from-bot"}}),
    );
    let card = b.s.cards.wait_for(1).await;
    assert_eq!(card.secret_inputs.len(), 1, "{card:?}");
    assert_eq!(card.secret_inputs[0].name, "token");
    assert!(card.secret_inputs[0].required);

    // 缺必填项：报错，卡片还等着。
    let err =
        b.s.answer_with_secrets(&card.id, Answer::Approve, Default::default())
            .unwrap_err();
    assert!(err.to_string().contains("token"), "{err}");
    assert!(!b.installed("app.input"));

    b.s.answer_with_secrets(
        &card.id,
        Answer::Approve,
        [(
            "token".to_owned(),
            wonton_protocol::SecretString::new("s3cret"),
        )]
        .into(),
    )
    .unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.installed("app.input"));
}

/// 最高级权限的两张卡：密钥填在第一张上，执行用的是第一张卡的值（第二张卡没有输入）。
#[tokio::test]
async fn a_highest_install_takes_the_secrets_from_the_first_card() {
    let b = bench().await;
    let source = official_with_mount("app.example.root", "全能", "docker.socket")
        .replace("spec: '0.4'", "spec: '0.8'")
        .replace(
            "    volumes:",
            "    environment:\n      TOKEN: ${WONTON_SECRET_TOKEN}\n    volumes:",
        )
        .replace(
            "permissions: [docker.socket]",
            "permissions: [docker.socket]\n  secrets:\n    token:\n      input:\n        title: {zh: 令牌, en: Token}\n        required: true",
        );
    let package =
        crate::plugin::PluginPackage::parse(&source, &Languages::from_header(Some("zh-Hans")))
            .unwrap();
    let entry = crate::plugin::storefront::index::IndexEntry::of(
        &package,
        "github:example/root",
        &"a".repeat(40),
    )
    .unwrap();
    entry
        .write_to(&b.s.dir.path().join("index"), &package)
        .unwrap();

    let call = b.s.spawn(INSTALL, json!({"id": "app.example.root"}));
    let card = b.s.cards.wait_for(1).await;
    assert_eq!(card.secret_inputs.len(), 1, "{card:?}");
    b.s.answer_with_secrets(
        &card.id,
        Answer::Approve,
        [(
            "token".to_owned(),
            wonton_protocol::SecretString::new("s3cret"),
        )]
        .into(),
    )
    .unwrap();
    let card = b.s.cards.wait_for(2).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.installed("app.example.root"));
}
