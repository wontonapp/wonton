//! 跳数的行为测试（ADR-0045 的“频率和跳数”）：只数不同 Bot 之间的来回，到顶时主人收到“已停下”卡片，选“继续”
//! 或有人说话就清零。网关后面接假的 Tuwunel，时钟是注入的。

use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt as _;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use wonton_protocol::{
    Answer, BotKeeper, ConfirmationCard, ConfirmationRequester, ConfirmationState,
    DeviceConfirmationPush, HOP_LIMIT_ERRCODE,
};

use super::family_rules_tests::{Home, ROOM};
use super::send_limits::SendLimits;
use crate::mcp::RecipientPush;

/// 两个 Bot 轮流说话，每条隔 3 秒（频率不挡）：`hermes` 先说，`from..to` 条全都放行。
async fn take_turns(home: &Home, from: usize, to: usize) {
    for n in from..to {
        home.tick(3000);
        let txn = format!("t{n}");
        let (status, _, body) = if n % 2 == 0 {
            home.bot_says(ROOM, &txn).await
        } else {
            home.other_says(ROOM, &txn).await
        };
        assert_eq!(status, StatusCode::OK, "第 {n} 条：{body}");
    }
}

/// 等主人的卡片 `id` 结束成 `done` 这一类：卡片的任务这时已经决定清不清零了。
async fn wait_settled(home: &Home, id: &str, done: fn(&ConfirmationState) -> bool) {
    for _ in 0..500 {
        if home.cards.states(id).iter().any(done) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等了 5 秒卡片还没结束：{:?}", home.cards.states(id));
}

async fn wait_resumed(home: &Home, id: &str) {
    wait_settled(home, id, |state| {
        matches!(state, ConfirmationState::Succeeded { .. })
    })
    .await;
}

/// 等家人 `alice` 的“等你确认”列表里出现卡片。
async fn alices_card(home: &Home) -> ConfirmationCard {
    for _ in 0..500 {
        if let Some(card) = home.bus.confirmations().pending_for(Some("alice")).pop() {
            return card;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等了 5 秒 alice 还没收到卡片");
}

fn requested_by(card: &ConfirmationCard) -> &str {
    match &card.requester {
        Some(ConfirmationRequester::Bot { user_id, .. }) => user_id,
        other => panic!("卡片的发起者不是 Bot：{other:?}"),
    }
}

fn alice() -> BotKeeper {
    BotKeeper::User {
        username: "alice".to_owned(),
    }
}

/// 来回到顶：`hermes` 和 `claw` 轮流说 21 条（20 跳），第 22 条换人时被挡，主人收到卡片。
async fn stop_the_pair(home: &Home) -> ConfirmationCard {
    take_turns(home, 0, 21).await;
    home.tick(3000);
    let (status, headers, body) = home.other_says(ROOM, "over").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["errcode"], HOP_LIMIT_ERRCODE);
    assert_eq!(home.refusal(&headers), Some("hop_limit"));
    home.cards.wait_for(1).await
}

#[tokio::test]
async fn one_bot_talking_alone_is_never_stopped() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    // 一个 Agent 忙起来：50 条进度、回复和编辑，人一直不说话。
    for n in 0..50 {
        home.tick(3000);
        let txn = format!("t{n}");
        let (status, _, body) = if n % 3 == 0 {
            home.other_says(ROOM, &txn).await
        } else {
            home.other_sends(
                ROOM,
                &txn,
                json!({
                    "msgtype": "m.text",
                    "body": "* 进度 50%",
                    "m.new_content": {"msgtype": "m.text", "body": "进度 50%"},
                    "m.relates_to": {"rel_type": "m.replace", "event_id": "$progress"},
                }),
            )
            .await
        };
        assert_eq!(status, StatusCode::OK, "第 {n} 条：{body}");
    }
    assert_eq!(home.fake.state().sent.len(), 50, "都到了上游");
    assert!(home.cards.posted.lock().unwrap().is_empty(), "不该问主人");
}

#[tokio::test]
async fn edits_between_two_bots_are_not_turns() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    // `claw` 一直改自己的进度消息，`hermes` 在中间说话：编辑不算一跳，也不算“上一条是 claw 说的”。
    for n in 0..60 {
        home.tick(3000);
        let txn = format!("t{n}");
        let (status, _, body) = if n % 2 == 0 {
            home.bot_says(ROOM, &txn).await
        } else {
            home.other_sends(
                ROOM,
                &txn,
                json!({
                    "algorithm": "m.megolm.v1.aes-sha2",
                    "ciphertext": "…",
                    "m.relates_to": {"rel_type": "m.replace", "event_id": "$progress"},
                }),
            )
            .await
        };
        assert_eq!(status, StatusCode::OK, "第 {n} 条：{body}");
    }
}

#[tokio::test]
async fn two_bots_going_back_and_forth_are_stopped_and_the_owner_is_asked() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    let card = stop_the_pair(&home).await;
    assert_eq!(home.fake.state().sent.len(), 21, "被挡的那条没到上游");
    assert!(card.title.contains("@hermes:test"), "{}", card.title);
    assert!(card.title.contains("@claw:test"), "{}", card.title);
    assert!(
        card.choices.iter().any(|c| c.answer == Answer::Approve)
            && card.choices.iter().any(|c| c.answer == Answer::Reject)
    );
    assert_eq!(home.bus.confirmations().pending().len(), 1);

    // 上一条是 hermes 说的：它接着说不算一跳，照样放行。
    home.tick(3000);
    let (status, _, body) = home.bot_says(ROOM, "same-bot").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // 再换人还是挡，同一回合不再发第二张卡片。
    home.tick(3000);
    let (status, _, _) = home.other_says(ROOM, "again").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(home.cards.posted.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn choosing_continue_lets_them_carry_on() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    let card = stop_the_pair(&home).await;
    home.bus
        .confirmations()
        .answer(&card.id, Answer::Approve, None)
        .expect("主人选继续");
    wait_resumed(&home, &card.id).await;

    // 清零了：又能来回 20 跳（最后一条是 claw 说的），hermes 再接话时到顶，再问一次。
    home.tick(3000);
    let (status, _, body) = home.other_says(ROOM, "resumed").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for n in 0..20 {
        home.tick(3000);
        let txn = format!("r{n}");
        let (status, _, body) = if n % 2 == 0 {
            home.bot_says(ROOM, &txn).await
        } else {
            home.other_says(ROOM, &txn).await
        };
        assert_eq!(status, StatusCode::OK, "第 {n} 条：{body}");
    }
    home.tick(3000);
    let (status, _, _) = home.bot_says(ROOM, "over-again").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    home.cards.wait_for(2).await;
}

#[tokio::test]
async fn keeping_it_stopped_leaves_the_room_stopped_until_someone_speaks() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    let card = stop_the_pair(&home).await;
    home.bus
        .confirmations()
        .answer(&card.id, Answer::Reject, None)
        .expect("主人选先停着");
    home.tick(3000);
    let (status, _, _) = home.other_says(ROOM, "still").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 人说一句就清零。
    home.tick(3000);
    let (status, _, body) = home.human_says(ROOM, "from-human").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    take_turns(&home, 100, 121).await;
}

#[tokio::test]
async fn an_hour_without_messages_starts_the_room_over() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    stop_the_pair(&home).await;
    home.tick(3_600_000);
    let (status, _, body) = home.other_says(ROOM, "an-hour-later").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_configured_hop_limit_is_enforced() {
    let home = Home::start(SendLimits {
        hop_limit: 5,
        ..SendLimits::default()
    })
    .await;
    home.seed_room(ROOM);
    // 最后一条是 claw 说的（5 跳），hermes 再接话就是第 6 跳。
    take_turns(&home, 0, 6).await;
    home.tick(3000);
    let (status, headers, _) = home.bot_says(ROOM, "sixth-hop").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(home.refusal(&headers), Some("hop_limit"));
    let card = home.cards.wait_for(1).await;
    assert!(
        card.title.contains('5'),
        "卡片写的是配置里的 5：{}",
        card.title
    );
}

#[tokio::test]
async fn each_keeper_gets_a_card_in_the_stopped_room() {
    let home = Home::with_keeper(SendLimits::default(), alice()).await;
    home.seed_room(ROOM);
    let owners = stop_the_pair(&home).await;
    let alices = alices_card(&home).await;
    assert_ne!(owners.id, alices.id);
    assert_eq!(
        requested_by(&owners),
        "@hermes:test",
        "主人的卡片以他的 Bot 的名义"
    );
    assert_eq!(
        requested_by(&alices),
        "@claw:test",
        "alice 的卡片以她的 Bot 的名义"
    );
    for card in [&owners, &alices] {
        assert_eq!(card.origin.as_ref().map(|o| o.room_id.as_str()), Some(ROOM));
    }
    // 各看各的：主人的列表里没有 alice 的卡片。
    let owner_list = home.bus.confirmations().pending_for(None);
    assert_eq!(owner_list.len(), 1);
    assert_eq!(owner_list[0].id, owners.id);
    assert_eq!(
        home.cards.posted.lock().unwrap().len(),
        1,
        "alice 的卡片不进主人的通知房间"
    );
}

#[tokio::test]
async fn a_shared_bot_and_the_owners_bot_share_one_card() {
    let home = Home::with_keeper(SendLimits::default(), BotKeeper::Household).await;
    home.seed_room(ROOM);
    stop_the_pair(&home).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(home.cards.posted.lock().unwrap().len(), 1);
    assert_eq!(home.bus.confirmations().pending_for(None).len(), 1);
}

#[tokio::test]
async fn both_keepers_must_choose_continue() {
    let home = Home::with_keeper(SendLimits::default(), alice()).await;
    home.seed_room(ROOM);
    let owners = stop_the_pair(&home).await;
    let alices = alices_card(&home).await;
    let confirmations = home.bus.confirmations();
    confirmations
        .answer(&owners.id, Answer::Approve, None)
        .expect("主人选继续");
    home.tick(3000);
    let (status, _, _) = home.other_says(ROOM, "only-one-said-yes").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "alice 还没回答");
    confirmations
        .answer(&alices.id, Answer::Approve, None)
        .expect("alice 选继续");
    wait_resumed(&home, &owners.id).await;
    home.tick(3000);
    let (status, _, body) = home.other_says(ROOM, "both-said-yes").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn one_keeper_keeping_it_stopped_keeps_the_room_stopped() {
    let home = Home::with_keeper(SendLimits::default(), alice()).await;
    home.seed_room(ROOM);
    let owners = stop_the_pair(&home).await;
    let alices = alices_card(&home).await;
    let confirmations = home.bus.confirmations();
    confirmations
        .answer(&owners.id, Answer::Approve, None)
        .expect("主人选继续");
    confirmations
        .answer(&alices.id, Answer::Reject, None)
        .expect("alice 选先停着");
    wait_settled(&home, &owners.id, |state| {
        matches!(state, ConfirmationState::Failed { .. })
    })
    .await;
    home.tick(3000);
    let (status, _, _) = home.other_says(ROOM, "still-stopped").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// 某台设备在卡片 `id` 上选“继续”。
async fn approve_as(
    home: &Home,
    device: iroh::EndpointId,
    id: &str,
) -> (StatusCode, hyper::HeaderMap, Value) {
    let path = format!("/_wonton/confirmations/{id}");
    home.core_as(device, Method::POST, &path, json!({"answer": "approve"}))
        .await
}

#[tokio::test]
async fn keepers_only_see_and_answer_their_own_cards() {
    let home = Home::with_keeper(SendLimits::default(), alice()).await;
    home.seed_room(ROOM);
    let owners = stop_the_pair(&home).await;
    let alices = alices_card(&home).await;
    let hers = home.member_device("alice");
    let list = |device| {
        home.core_as(
            device,
            Method::GET,
            "/_wonton/api/confirmations",
            Value::Null,
        )
    };
    let ids = |body: &Value| -> Vec<String> {
        body["confirmations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|card| card["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let (status, _, body) = list(hers).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ids(&body), vec![alices.id.clone()]);
    let (status, _, body) = list(home.phone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ids(&body), vec![owners.id.clone()]);

    let (status, _, body) = approve_as(&home, hers, &owners.id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["kind"], "not_yours", "{body}");
    let (status, _, body) = approve_as(&home, home.phone(), &alices.id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _, body) = approve_as(&home, hers, &alices.id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// 收件人推送的替身：记下推给了谁、哪张卡（ADR-0016）。
#[derive(Default)]
struct RecordedPushes(std::sync::Mutex<Vec<(String, DeviceConfirmationPush)>>);

impl RecipientPush for RecordedPushes {
    fn push<'a>(
        &'a self,
        person: &'a str,
        card: &'a DeviceConfirmationPush,
    ) -> futures_util::future::BoxFuture<'a, Result<(), String>> {
        async move {
            self.0
                .lock()
                .unwrap()
                .push((person.to_owned(), card.clone()));
            Ok(())
        }
        .boxed()
    }
}

#[tokio::test]
async fn the_family_card_is_pushed_to_her_and_the_owners_is_not() {
    let home = Home::with_keeper(SendLimits::default(), alice()).await;
    home.seed_room(ROOM);
    let pushes = Arc::new(RecordedPushes::default());
    home.bus.confirmations().attach_push(Some(pushes.clone()));
    let owners = stop_the_pair(&home).await;
    let alices = alices_card(&home).await;
    let recorded = pushes.0.lock().unwrap();
    assert_eq!(recorded.len(), 1, "只有收件人的卡片走直推：{recorded:?}");
    let (person, card) = &recorded[0];
    assert_eq!(person, "alice");
    assert_eq!(card.id, alices.id);
    assert_ne!(alices.id, owners.id, "两边的卡片不是同一张");
    assert_eq!(card.title, alices.title);
    assert_eq!(card.origin.as_ref().map(|o| o.room_id.as_str()), Some(ROOM));
    drop(recorded);
    // 主人的卡片照常进通知房间（消息推送自己会响），不走直推。
    assert_eq!(home.cards.posted.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_family_card_is_written_in_her_language() {
    let home = Home::with_keeper(SendLimits::default(), alice()).await;
    home.seed_room(ROOM);
    // alice 的 App 用日语（ADR-0043）：她的卡片按她的语言写，主人的照旧按主人的（这里记成中文）。
    home.plugins.note_person_languages("alice", Some("ja"));
    home.plugins.note_person_languages("owner", Some("zh-Hans"));
    let owners = stop_the_pair(&home).await;
    let alices = alices_card(&home).await;
    assert!(
        alices.title.contains("やり取りしたため"),
        "alice 的卡片是日语：{}",
        alices.title
    );
    assert!(
        alices.choices.iter().any(|choice| choice.label == "続ける"),
        "alice 的按钮是日语：{:?}",
        alices.choices
    );
    assert!(
        owners.title.contains("来回说了"),
        "主人的卡片还是中文：{}",
        owners.title
    );
    assert!(
        owners.choices.iter().any(|choice| choice.label == "继续"),
        "主人的按钮还是中文：{:?}",
        owners.choices
    );
}
