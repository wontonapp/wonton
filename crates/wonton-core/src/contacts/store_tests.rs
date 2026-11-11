use super::{RemoteRequest, RemoteResult, Stored, StoredContact, Take};
use wonton_protocol::{BotRef, ContactKind, ContactOutcome, ContactState};

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: "hermes".to_owned(),
        bot: name.to_owned(),
    }
}

fn at() -> u64 {
    1_000
}

fn remote_server() -> &'static str {
    "remote.example"
}

#[test]
fn a_remote_request_is_recorded_as_incoming() {
    let mut stored = Stored::default();
    let take = stored.on_remote_request(
        &RemoteRequest {
            bot: &bot("default"),
            from_user: "@helper:remote.example",
            from_server: remote_server(),
            request_id: "req-1",
            display_name: Some("Helper".to_owned()),
            keeper_display_name: Some("Keeper".to_owned()),
        },
        at(),
    );
    assert_eq!(take, Take::Recorded);
    let row = stored
        .row(&bot("default"), "@helper:remote.example")
        .unwrap();
    assert_eq!(row.state, ContactState::Incoming);
    assert_eq!(row.kind, ContactKind::Bot);
    assert_eq!(row.remote.as_deref(), Some(remote_server()));
    assert_eq!(row.request_id.as_deref(), Some("req-1"));
    assert_eq!(row.display_name.as_deref(), Some("Helper"));
    assert_eq!(row.keeper_display_name.as_deref(), Some("Keeper"));
}

#[test]
fn a_repeated_request_updates_the_same_row() {
    let mut stored = Stored::default();
    stored.on_remote_request(
        &RemoteRequest {
            bot: &bot("default"),
            from_user: "@helper:remote.example",
            from_server: remote_server(),
            request_id: "req-1",
            display_name: None,
            keeper_display_name: None,
        },
        at(),
    );
    stored.on_remote_request(
        &RemoteRequest {
            bot: &bot("default"),
            from_user: "@helper:remote.example",
            from_server: remote_server(),
            request_id: "req-2",
            display_name: Some("New name".to_owned()),
            keeper_display_name: None,
        },
        at() + 5,
    );
    let rows = stored.rows(&bot("default"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id.as_deref(), Some("req-2"));
    assert_eq!(rows[0].display_name.as_deref(), Some("New name"));
    assert_eq!(rows[0].updated_at_ms, at() + 5);
}

#[test]
fn a_request_between_active_contacts_is_ignored() {
    let mut stored = Stored::default();
    stored.on_remote_request(
        &RemoteRequest {
            bot: &bot("default"),
            from_user: "@helper:remote.example",
            from_server: remote_server(),
            request_id: "req-1",
            display_name: None,
            keeper_display_name: None,
        },
        at(),
    );
    stored
        .approve(&bot("default"), "@helper:remote.example", at())
        .unwrap();
    let take = stored.on_remote_request(
        &RemoteRequest {
            bot: &bot("default"),
            from_user: "@helper:remote.example",
            from_server: remote_server(),
            request_id: "req-2",
            display_name: None,
            keeper_display_name: None,
        },
        at(),
    );
    assert_eq!(take, Take::Ignore);
    assert_eq!(stored.rows(&bot("default"))[0].state, ContactState::Active);
}

#[test]
fn requests_in_both_directions_establish_directly() {
    let mut stored = Stored::default();
    // 我方先发了请求（Pending），对方也发了请求过来：直接成立，我方还要回一条同意的结果。
    stored.upsert(
        &bot("default"),
        contact(
            "@helper:remote.example",
            ContactKind::Bot,
            ContactState::Pending,
            Some("mine-1".to_owned()),
        ),
    );
    let take = stored.on_remote_request(
        &RemoteRequest {
            bot: &bot("default"),
            from_user: "@helper:remote.example",
            from_server: remote_server(),
            request_id: "theirs-1",
            display_name: None,
            keeper_display_name: None,
        },
        at(),
    );
    assert_eq!(take, Take::Established);
    assert_eq!(stored.rows(&bot("default"))[0].state, ContactState::Active);
}

#[test]
fn a_result_completes_the_pending_request() {
    let mut stored = Stored::default();
    stored.upsert(
        &bot("default"),
        pending_request("mine-1", "@helper:remote.example"),
    );
    let applied = stored.on_remote_result(
        &RemoteResult {
            bot: &bot("default"),
            from_user: "@helper:remote.example",
            from_server: remote_server(),
            request_id: "mine-1",
            outcome: ContactOutcome::Approved,
            keeper_display_name: Some("Keeper".to_owned()),
        },
        at(),
    );
    assert_eq!(applied, Some(ContactState::Active));
    let row = stored
        .row(&bot("default"), "@helper:remote.example")
        .unwrap();
    assert_eq!(row.state, ContactState::Active);
    assert_eq!(row.keeper_display_name.as_deref(), Some("Keeper"));
}

#[test]
fn a_result_for_an_unknown_request_is_ignored() {
    let mut stored = Stored::default();
    stored.upsert(
        &bot("default"),
        pending_request("mine-1", "@helper:remote.example"),
    );
    // 删了再请求后，旧请求迟到的结果不算数。
    assert_eq!(
        stored.on_remote_result(
            &RemoteResult {
                bot: &bot("default"),
                from_user: "@helper:remote.example",
                from_server: remote_server(),
                request_id: "stale",
                outcome: ContactOutcome::Approved,
                keeper_display_name: None,
            },
            at(),
        ),
        None
    );
}

#[test]
fn a_result_for_another_bot_is_ignored() {
    let mut stored = Stored::default();
    stored.upsert(
        &bot("default"),
        pending_request("mine-1", "@helper:remote.example"),
    );
    stored.upsert(
        &bot("side"),
        pending_request("mine-2", "@helper:remote.example"),
    );
    // 结果事件里的 `to` 指着另一个 Bot：这一行不该被改（审查 #2）。
    assert_eq!(
        stored.on_remote_result(
            &RemoteResult {
                bot: &bot("side"),
                from_user: "@helper:remote.example",
                from_server: remote_server(),
                request_id: "mine-1",
                outcome: ContactOutcome::Approved,
                keeper_display_name: None,
            },
            at(),
        ),
        None
    );
    assert_eq!(
        stored
            .row(&bot("default"), "@helper:remote.example")
            .unwrap()
            .state,
        ContactState::Pending
    );
    // 发送方服务器对不上（冒充）同样忽略。
    assert_eq!(
        stored.on_remote_result(
            &RemoteResult {
                bot: &bot("default"),
                from_user: "@helper:remote.example",
                from_server: "elsewhere.example",
                request_id: "mine-1",
                outcome: ContactOutcome::Approved,
                keeper_display_name: None,
            },
            at(),
        ),
        None
    );
}

#[test]
fn rejected_and_missing_outcomes_land_in_their_states() {
    let mut stored = Stored::default();
    stored.upsert(&bot("a"), pending_request("mine-1", "@x:remote.example"));
    stored.upsert(&bot("b"), pending_request("mine-2", "@y:remote.example"));
    stored.on_remote_result(
        &RemoteResult {
            bot: &bot("a"),
            from_user: "@x:remote.example",
            from_server: "remote.example",
            request_id: "mine-1",
            outcome: ContactOutcome::Rejected,
            keeper_display_name: None,
        },
        at(),
    );
    stored.on_remote_result(
        &RemoteResult {
            bot: &bot("b"),
            from_user: "@y:remote.example",
            from_server: "remote.example",
            request_id: "mine-2",
            outcome: ContactOutcome::NotFound,
            keeper_display_name: None,
        },
        at(),
    );
    assert_eq!(
        stored.row(&bot("a"), "@x:remote.example").unwrap().state,
        ContactState::Declined
    );
    assert_eq!(
        stored.row(&bot("b"), "@y:remote.example").unwrap().state,
        ContactState::NotFound
    );
}

#[test]
fn a_remote_remove_deletes_the_row_and_reports_the_bot() {
    let mut stored = Stored::default();
    stored.upsert(
        &bot("default"),
        pending_request("mine-1", "@helper:remote.example"),
    );
    let removed =
        stored.on_remote_remove(&bot("default"), "@helper:remote.example", "remote.example");
    assert!(removed.is_some());
    assert!(
        stored
            .row(&bot("default"), "@helper:remote.example")
            .is_none()
    );
    assert_eq!(
        stored.on_remote_remove(
            &bot("default"),
            "@stranger:remote.example",
            "remote.example"
        ),
        None
    );
}

#[test]
fn approving_needs_an_incoming_row_and_reject_deletes_it() {
    let mut stored = Stored::default();
    assert!(
        stored
            .approve(&bot("default"), "@helper:remote.example", at())
            .is_err()
    );
    stored.on_remote_request(
        &RemoteRequest {
            bot: &bot("default"),
            from_user: "@helper:remote.example",
            from_server: remote_server(),
            request_id: "req-1",
            display_name: None,
            keeper_display_name: None,
        },
        at(),
    );
    stored
        .approve(&bot("default"), "@helper:remote.example", at())
        .unwrap();
    assert_eq!(stored.rows(&bot("default"))[0].state, ContactState::Active);
    assert!(
        stored
            .approve(&bot("default"), "@helper:remote.example", at())
            .is_err()
    );
    stored.on_remote_request(
        &RemoteRequest {
            bot: &bot("other"),
            from_user: "@helper:remote.example",
            from_server: remote_server(),
            request_id: "req-2",
            display_name: None,
            keeper_display_name: None,
        },
        at(),
    );
    stored.remove(&bot("other"), "@helper:remote.example");
    assert!(stored.rows(&bot("other")).is_empty());
}

#[test]
fn a_person_joins_out_of_pending_only() {
    let mut stored = Stored::default();
    let mut row = pending_request("mine-1", "@guest:remote.example");
    row.kind = ContactKind::Person;
    row.room = Some("!chat:local".to_owned());
    stored.upsert(&bot("default"), row);
    stored
        .person_joined(&bot("default"), "@guest:remote.example", at())
        .unwrap();
    assert_eq!(stored.rows(&bot("default"))[0].state, ContactState::Active);

    let bot_row = pending_request("mine-2", "@helper:remote.example");
    stored.upsert(&bot("default"), bot_row.clone());
    // Bot 行不走人路径。
    assert!(
        stored
            .person_joined(&bot("default"), "@helper:remote.example", at())
            .is_err()
    );
    let _ = bot_row;
}

#[test]
fn the_store_round_trips_through_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("contacts.json");
    let mut stored = Stored::default();
    stored.upsert(
        &bot("default"),
        pending_request("mine-1", "@helper:remote.example"),
    );
    stored.set_peer_room(remote_server(), "!contact:remote.example");
    super::write(&path, &stored).unwrap();
    let read_back = super::read(&path).unwrap();
    assert_eq!(read_back, stored);
    assert_eq!(
        read_back.peer_room(remote_server()).as_deref(),
        Some("!contact:remote.example")
    );
}

fn pending_request(id: &str, user: &str) -> StoredContact {
    contact(
        user,
        ContactKind::Bot,
        ContactState::Pending,
        Some(id.to_owned()),
    )
}

fn contact(
    user: &str,
    kind: ContactKind,
    state: ContactState,
    request_id: Option<String>,
) -> StoredContact {
    StoredContact {
        user_id: user.to_owned(),
        kind,
        state,
        remote: Some(remote_server().to_owned()),
        display_name: None,
        keeper_display_name: None,
        request_id,
        room: None,
        updated_at_ms: at(),
    }
}
