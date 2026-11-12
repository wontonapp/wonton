use super::accepted_pair;
use serde_json::{Value, json};

fn membership(user: &str, state: &str) -> Value {
    json!({"type": "m.room.member", "state_key": user, "content": {"membership": state}})
}

#[test]
fn acceptance_requires_the_exact_peer_to_join_the_marked_room() {
    let own = "@alice:home.test";
    let peer = "@bob:friend.test";
    let marker =
        json!({"type": "app.wonton.friend", "state_key": "", "sender": own, "content": {}});
    let mut events = vec![membership(own, "join"), membership(peer, "invite"), marker];
    assert!(!accepted_pair(own, peer, &events));
    events[1] = membership(peer, "join");
    assert!(accepted_pair(own, peer, &events));
    assert!(!accepted_pair(own, "@bob:friend.test.evil.test", &events));
    events.push(membership("@third:friend.test", "join"));
    assert!(!accepted_pair(own, peer, &events));
    events.pop();
    events[1] = membership(peer, "leave");
    assert!(!accepted_pair(own, peer, &events));
}

#[test]
fn only_a_participant_unredacted_empty_key_marker_establishes_a_friend() {
    let own = "@alice:home.test";
    let peer = "@bob:friend.test";
    let mut events = vec![membership(own, "join"), membership(peer, "join")];
    assert!(!accepted_pair(own, peer, &events));
    for marker in [
        json!({"type": "app.wonton.friend", "state_key": "", "sender": "@third:friend.test", "content": {}}),
        json!({"type": "app.wonton.friend", "state_key": peer, "sender": own, "content": {}}),
        json!({"type": "app.wonton.friend", "state_key": "", "sender": own, "content": {}, "unsigned": {"redacted_because": {}}}),
    ] {
        events.push(marker);
        assert!(!accepted_pair(own, peer, &events));
        events.pop();
    }
    events
        .push(json!({"type": "app.wonton.friend", "state_key": "", "sender": peer, "content": {}}));
    assert!(accepted_pair(own, peer, &events));
}
