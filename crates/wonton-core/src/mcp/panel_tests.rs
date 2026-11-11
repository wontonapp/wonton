//! Bot 往面板钉页（#35）和 MCP Apps 的转发：房间不问、主面板问一次、上限、列表、取消、工具可见性。

use std::time::Duration;

use serde_json::{Value, json};
use wonton_protocol::{Answer, BotKeeper, CARD_FIELD, PanelScope, TOOL_CALL_USER_META};

use super::test_setup::setup_with_upstreams;
use super::upstream::Upstreams;
use super::{Bus, Caller};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::plugin::PluginId;
use crate::plugin::bots::BotEntry;

struct Bot {
    caller: Caller,
    id: PluginId,
    room: String,
}

impl Bot {
    /// 提供界面的插件。
    fn plugin(&self) -> PluginId {
        PluginId::parse("app.example.notes").unwrap()
    }
}

fn caller(plugin: &str) -> (Caller, PluginId) {
    let id = PluginId::parse(plugin).unwrap();
    let title = [("zh".to_owned(), "看板".to_owned())].into();
    (
        Caller::for_bot(
            id.clone(),
            Some(&title),
            "hermes".to_owned(),
            BotKeeper::User {
                username: "owner".into(),
            },
        ),
        id,
    )
}

async fn register(bus: &Bus, plugin: &PluginId, room: &str) {
    let mut bots = bus.plugins.store().bots().load(plugin).unwrap();
    bots.bots.insert(
        "hermes".into(),
        BotEntry {
            account: crate::matrix::BotAccount {
                user_id: "@hermes:test".to_owned(),
                home_room: room.to_owned(),
            },
            device_id: None,
            keeper: BotKeeper::User {
                username: "owner".into(),
            },
            bus_token: "token".into(),
        },
    );
    bus.plugins.store().bots().save(plugin, &bots).unwrap();
}

async fn attach_matrix(bus: &Bus, dir: &std::path::Path) -> crate::matrix::fake::Fake {
    let (fake, config) = crate::matrix::fake::start().await;
    let matrix = Matrix::connect(&config, dir, HttpClient::new())
        .await
        .unwrap();
    bus.attach_matrix(Some(std::sync::Arc::new(matrix)), "owner".into());
    fake
}

async fn bot(plugin: &str) -> (super::test_setup::Setup, Bot, crate::matrix::fake::Fake) {
    let setup = setup_with_upstreams(Upstreams::new()).await;
    let (caller, id) = caller(plugin);
    let room = "!hermes:wonton.localhost";
    register(&setup.bus, &id, room).await;
    let fake = attach_matrix(&setup.bus, &setup.dir.path().join("matrix")).await;
    fake.state().rooms.insert(
        room.to_owned(),
        crate::matrix::fake::Room {
            members: ["@hermes:test".to_owned(), "@owner:test".to_owned()]
                .into_iter()
                .collect(),
            invited: Default::default(),
            left: Default::default(),
            encrypted: true,
            room_type: None,
            creator: None,
            invited_by: Default::default(),
        },
    );
    let b = Bot {
        caller,
        id,
        room: room.to_owned(),
    };
    (setup, b, fake)
}

fn pin_args(plugin: &str, path: &str, target: &str) -> Value {
    json!({"plugin": plugin, "path": path, "target": target})
}

async fn wait_card(fake: &crate::matrix::fake::Fake) -> String {
    for _ in 0..500 {
        if let Some((_, content)) = fake
            .state()
            .sent
            .iter()
            .find(|(_, content)| content.get(CARD_FIELD).is_some())
        {
            return content[CARD_FIELD]["id"].as_str().unwrap().to_owned();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等了 5 秒还没有确认卡片");
}

async fn wait_for_card(
    fake: &crate::matrix::fake::Fake,
    call: &mut tokio::task::JoinHandle<rmcp::model::CallToolResult>,
) -> String {
    for _ in 0..500 {
        if call.is_finished() {
            panic!("调用先结束了：{:?}", call.await);
        }
        if let Some((_, content)) = fake
            .state()
            .sent
            .iter()
            .find(|(_, content)| content.get(CARD_FIELD).is_some())
        {
            return content[CARD_FIELD]["id"].as_str().unwrap().to_owned();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等了 5 秒还没有确认卡片");
}

#[tokio::test]
async fn a_bot_pins_into_its_own_room_without_asking() {
    let (s, b, _fake) = bot("com.example.agent").await;
    let result = s
        .call_as(
            &b.caller,
            "wonton__pin_page",
            pin_args("app.example.notes", "/one", "room"),
        )
        .await;
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    assert!(
        s.bus.plugins.store().pins().load(&b.id).unwrap().bots["hermes"]
            .pins
            .iter()
            .any(|pin| pin.scope
                == PanelScope::Room {
                    room_id: b.room.clone()
                })
    );
    // 自己的房间不问，通知房间和 Bot 房间都没有卡片。
    assert!(s.cards.posted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn re_pinning_updates_title_and_attention() {
    let (s, b, _fake) = bot("com.example.agent").await;
    let first = s
        .call_as(
            &b.caller,
            "wonton__pin_page",
            pin_args("app.example.notes", "/one", "room"),
        )
        .await;
    assert!(!first.is_error.unwrap_or(false));
    let id = {
        let record = s.bus.plugins.store().pins().load(&b.id).unwrap();
        record.bots["hermes"].pins[0].id.clone()
    };
    let second = s
        .call_as(
            &b.caller,
            "wonton__pin_page",
            json!({"plugin": "app.example.notes", "path": "/one", "target": "room",
                   "title": "今天", "attention": true}),
        )
        .await;
    assert!(!second.is_error.unwrap_or(false));
    let record = s.bus.plugins.store().pins().load(&b.id).unwrap();
    let pins = &record.bots["hermes"].pins;
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].id, id);
    assert_eq!(pins[0].title, "今天");
    assert!(pins[0].attention);
}

#[tokio::test]
async fn the_first_main_pin_asks_once() {
    let (s, b, fake) = bot("com.example.agent").await;
    let mut call = s.spawn_as(
        &b.caller,
        "wonton__pin_page",
        pin_args("app.example.notes", "/one", "main"),
    );
    let card = wait_for_card(&fake, &mut call).await;
    s.bus
        .confirmations()
        .answer(&card, Answer::Approve, None)
        .unwrap();
    let result = call.await.unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let record = s.bus.plugins.store().pins().load(&b.id).unwrap();
    assert!(record.bots["hermes"].main_consent);
    assert_eq!(record.bots["hermes"].pins.len(), 1);

    let again = s
        .call_as(
            &b.caller,
            "wonton__pin_page",
            pin_args("app.example.notes", "/two", "main"),
        )
        .await;
    assert!(!again.is_error.unwrap_or(false));
    let cards = fake
        .state()
        .sent
        .iter()
        .filter(|(_, content)| {
            content.get(CARD_FIELD).is_some() && content.get("m.relates_to").is_none()
        })
        .count();
    assert_eq!(cards, 1, "第二次不再问");
}

#[tokio::test]
async fn a_refused_main_pin_is_not_stored() {
    let (s, b, fake) = bot("com.example.agent").await;
    let mut call = s.spawn_as(
        &b.caller,
        "wonton__pin_page",
        pin_args("app.example.notes", "/one", "main"),
    );
    let card = wait_for_card(&fake, &mut call).await;
    s.bus
        .confirmations()
        .answer(&card, Answer::Reject, None)
        .unwrap();
    let result = call.await.unwrap();
    assert_eq!(
        result.meta.as_ref().unwrap().0["app.wonton/failure"],
        "owner_rejected"
    );
    assert!(
        s.bus.plugins.store().pins().load(&b.id).unwrap().bots["hermes"]
            .pins
            .is_empty()
    );
    assert!(!s.bus.plugins.store().pins().load(&b.id).unwrap().bots["hermes"].main_consent);
}

#[tokio::test]
async fn the_user_page_limit_counts_the_room_panel() {
    let (s, b, fake) = bot("com.example.agent").await;
    let pages: Vec<Value> = (0..5)
        .map(|i| json!({"tab": {"type": "url", "url": format!("https://example.com/{i}")}}))
        .collect();
    fake.state().room_account_data.insert(
        (
            "@owner:test".into(),
            b.room.clone(),
            "app.wonton.room_panel".into(),
        ),
        json!({"pages": pages}),
    );
    let full = s
        .call_as(
            &b.caller,
            "wonton__pin_page",
            pin_args("app.example.notes", "/one", "room"),
        )
        .await;
    assert_eq!(
        full.meta.as_ref().unwrap().0["app.wonton/failure"],
        "panel_full"
    );
    fake.state().room_account_data.insert(
        (
            "@owner:test".into(),
            b.room.clone(),
            "app.wonton.room_panel".into(),
        ),
        json!({"pages": pages[..4]}),
    );
    let open = s
        .call_as(
            &b.caller,
            "wonton__pin_page",
            pin_args("app.example.notes", "/one", "room"),
        )
        .await;
    assert!(!open.is_error.unwrap_or(false), "{open:?}");
}

#[tokio::test]
async fn list_and_unpin_work_and_404_for_unknown_pages() {
    let (s, b, _fake) = bot("com.example.agent").await;
    s.call_as(
        &b.caller,
        "wonton__pin_page",
        pin_args("app.example.notes", "/one", "room"),
    )
    .await;
    let listed = s
        .call_as(&b.caller, "wonton__list_pinned_pages", json!({}))
        .await;
    assert!(!listed.is_error.unwrap_or(false));
    let text = listed.content[0].as_text().map(|t| t.text.clone()).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["pins"].as_array().unwrap().len(), 1);
    assert_eq!(value["pins"][0]["name"], "看板");
    let removed = s
        .call_as(
            &b.caller,
            "wonton__unpin_page",
            pin_args("app.example.notes", "/one", "room"),
        )
        .await;
    assert!(!removed.is_error.unwrap_or(false));
    let again = s
        .call_as(
            &b.caller,
            "wonton__unpin_page",
            pin_args("app.example.notes", "/one", "room"),
        )
        .await;
    assert_eq!(
        again.meta.as_ref().unwrap().0["app.wonton/failure"],
        "not_found"
    );
}

#[tokio::test]
async fn only_bots_can_pin_and_urls_must_be_https() {
    let (s, b, _fake) = bot("com.example.agent").await;
    let plain = Caller::new(PluginId::parse("app.example.notes").unwrap(), None);
    let result = s
        .call_as(
            &plain,
            "wonton__pin_page",
            json!({"url": "https://example.com/"}),
        )
        .await;
    assert_eq!(
        result.meta.as_ref().unwrap().0["app.wonton/failure"],
        "invalid_input"
    );
    let insecure = s
        .call_as(
            &b.caller,
            "wonton__pin_page",
            json!({"url": "http://example.com/", "target": "room"}),
        )
        .await;
    assert_eq!(
        insecure.meta.as_ref().unwrap().0["app.wonton/failure"],
        "invalid_input"
    );
}

#[tokio::test]
async fn cancelling_the_pin_call_still_asks_and_pins() {
    let (s, b, fake) = bot("com.example.agent").await;
    let handle = s.spawn_as(
        &b.caller,
        "wonton__pin_page",
        pin_args("app.example.notes", "/one", "main"),
    );
    let card = wait_card(&fake).await;
    handle.abort();
    s.bus
        .confirmations()
        .answer(&card, Answer::Approve, None)
        .unwrap();
    for _ in 0..200 {
        if s.bus.plugins.store().pins().load(&b.id).unwrap().bots["hermes"].main_consent {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("取消后没有接着钉上");
}

#[tokio::test]
async fn app_calls_run_as_the_owner() {
    let (s, b, _fake) = bot("com.example.agent").await;
    s.call_as(
        &b.caller,
        "wonton__pin_page",
        json!({"plugin": "app.example.notes", "resource_uri": "ui://monitor", "target": "room"}),
    )
    .await;
    let plugin = b.plugin();
    let bus = s.bus.clone();
    let call = tokio::spawn(async move {
        bus.app_call(
            &plugin,
            "delete",
            json!({"name": "x"}).as_object().cloned().unwrap(),
        )
        .await
    });
    let card = s.cards.wait_for(1).await;
    assert!(card.title.contains("你"), "{}", card.title);
    s.bus
        .confirmations()
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    assert!(call.await.unwrap().is_ok());
    let metas = s.notes.received_meta.lock().unwrap();
    assert_eq!(
        metas.last().unwrap().as_ref().unwrap().0[TOOL_CALL_USER_META],
        "owner"
    );
}

#[tokio::test]
async fn only_tools_declared_for_the_app_are_callable() {
    let (s, b, _fake) = bot("com.example.agent").await;
    s.call_as(
        &b.caller,
        "wonton__pin_page",
        json!({"plugin": "app.example.notes", "resource_uri": "ui://monitor", "target": "room"}),
    )
    .await;
    let tools = s.bus.app_tools(&b.plugin()).await.unwrap();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, ["search", "delete"]);

    let called = s
        .bus
        .app_call(
            &b.plugin(),
            "search",
            json!({"name": "x"}).as_object().cloned().unwrap(),
        )
        .await
        .unwrap();
    assert!(!called.is_error.unwrap_or(false));
    let refused = s
        .bus
        .app_call(
            &b.plugin(),
            "delete",
            json!({"name": "x"}).as_object().cloned().unwrap(),
        )
        .await
        .unwrap();
    // delete 声明给了界面，但没标只读：要确认，没人回答时结果是一条错误，不是调度错误。
    assert_eq!(refused.is_error, Some(true));
}

#[tokio::test]
async fn app_resources_keep_their_meta() {
    let (s, b, _fake) = bot("com.example.agent").await;
    s.call_as(
        &b.caller,
        "wonton__pin_page",
        json!({"plugin": "app.example.notes", "resource_uri": "ui://monitor", "target": "room"}),
    )
    .await;
    let result = s.bus.app_read(&b.plugin(), "ui://monitor").await.unwrap();
    let resource = &result.contents[0];
    assert!(matches!(
        resource,
        rmcp::model::ResourceContents::TextResourceContents { .. }
    ));
    let meta = match resource {
        rmcp::model::ResourceContents::TextResourceContents { meta, .. } => meta.as_ref().unwrap(),
        _ => unreachable!(),
    };
    assert_eq!(
        meta.0["ui"]["csp"]["connectDomains"][0],
        "https://api.example.com"
    );
}
