//! 跳数的行为测试（ADR-0045 的“频率和跳数”）：只数不同 Bot 之间的来回，到顶时主人收到“已停下”卡片，选“继续”
//! 或有人说话就清零。网关后面接假的 Tuwunel，时钟是注入的。

use std::time::Duration;

use hyper::StatusCode;
use serde_json::json;
use wonton_protocol::{Answer, ConfirmationCard, ConfirmationState, HOP_LIMIT_ERRCODE};

use super::family_rules_tests::{Home, ROOM};
use super::send_limits::SendLimits;

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

/// 等卡片 `id` 变成“已继续”：卡片的任务这时已经清零了。
async fn wait_resumed(home: &Home, id: &str) {
    for _ in 0..500 {
        let states = home.cards.states(id);
        if states
            .iter()
            .any(|state| matches!(state, ConfirmationState::Succeeded { .. }))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等了 5 秒卡片还没变成“已继续”：{:?}", home.cards.states(id));
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
