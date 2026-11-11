//! 联邦联系人事件校验的纯函数测试：邀请的房间类型和发起人对不上的一律不认，事件三方的核对同款。

use serde_json::json;

use super::exchange::{check_parties, invite_of_peer};

fn paired(names: &[&str]) -> impl Fn(&str) -> bool + '_ {
    let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    move |server: &str| names.iter().any(|n| n == server)
}

/// 收到的邀请房间状态的 stripped 形状（`invite_state.events`）。
fn invite_state(create_type: Option<&str>, inviter: &str) -> serde_json::Value {
    let mut content = json!({});
    if let Some(t) = create_type {
        content["type"] = json!(t);
    }
    json!({
        "invite_state": {
            "events": [
                { "type": "m.room.create", "sender": inviter, "content": content },
                { "type": "m.room.member", "sender": inviter,
                  "content": { "membership": "invite" } },
            ]
        }
    })
}

const ROOM_TYPE: &str = super::exchange::CONTACT_ROOM_TYPE;

#[test]
fn invite_of_peer_accepts_valid_invite() {
    let state = invite_state(Some(ROOM_TYPE), "@wonton-core:other.example");
    assert_eq!(
        invite_of_peer(&paired(&["other.example"]), &state).as_deref(),
        Some("other.example")
    );
}

#[test]
fn invite_of_peer_rejects_non_invites_and_unpaired() {
    // 房间类型不对（普通聊天房间）。
    let state = invite_state(None, "@wonton-core:other.example");
    assert_eq!(invite_of_peer(&paired(&["other.example"]), &state), None);
    // 发起人不是配对服务器上的核心账号。
    let state = invite_state(Some(ROOM_TYPE), "@stranger:other.example");
    assert_eq!(invite_of_peer(&paired(&["other.example"]), &state), None);
    // 服务器没配对。
    let state = invite_state(Some(ROOM_TYPE), "@wonton-core:other.example");
    assert_eq!(invite_of_peer(&paired(&[]), &state), None);
}

#[test]
fn check_parties_accepts_matching_request() {
    assert_eq!(
        check_parties(
            "home.test",
            &paired(&["other.example"]),
            "@wonton-core:other.example",
            "@helper:other.example",
            "@a:home.test",
        )
        .as_deref(),
        Some("other.example")
    );
}

#[test]
fn check_parties_rejects_mismatches() {
    // 发送者不是 from 用户所在服务器的核心账号。
    assert_eq!(
        check_parties(
            "home.test",
            &paired(&["other.example"]),
            "@wonton-core:elsewhere.example",
            "@helper:other.example",
            "@a:home.test",
        ),
        None
    );
    // from 的服务器没配对。
    assert_eq!(
        check_parties(
            "home.test",
            &paired(&[]),
            "@wonton-core:other.example",
            "@helper:other.example",
            "@a:home.test",
        ),
        None
    );
    // 收件人不在本机。
    assert_eq!(
        check_parties(
            "home.test",
            &paired(&["other.example"]),
            "@wonton-core:other.example",
            "@helper:other.example",
            "@other:other.example",
        ),
        None
    );
}
