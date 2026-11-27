use axum::Router;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use wonton_protocol::{
    BotKeeper, BotRef, MY_PRIVATE_BOT_PATH, OwnPrivateBotResponse, OwnPrivateBotResult,
};

use crate::gateway::family_plugin_tests::app_with_contact_facts;
use crate::gateway::family_tests::{bearer, send_as};
use crate::gateway::tests::{CORE, Harness, Reply};
use crate::members::MemberRecord;
use crate::plugin::PluginId;

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: "app.whoami".into(),
        bot: name.into(),
    }
}

struct Rig {
    home: Harness,
    app: Router,
    _matrix_state: tempfile::TempDir,
    device: iroh::EndpointId,
}

impl Rig {
    async fn start(wants: bool, ready: Option<BotRef>, reservation: Option<BotRef>) -> Self {
        let home = Harness::start(None).await;
        home.members
            .invite(MemberRecord {
                invited_by: None,
                role_id: "family".into(),
                wants_private_bot: wants,
                private_bot: ready,
                ..MemberRecord::for_test("mia")
            })
            .unwrap();
        home.members.mark_joined("mia").unwrap();
        if let Some(reservation) = reservation {
            home.members
                .reserve_private_bot("mia", &reservation)
                .unwrap();
        }
        let device = SecretKey::generate().public();
        home.tunnel
            .devices
            .add_if(device, None, Some("mia".into()), |_| true)
            .unwrap();
        let (app, matrix_state) = app_with_contact_facts(&home).await;
        Self {
            home,
            app,
            _matrix_state: matrix_state,
            device,
        }
    }

    async fn read(&self, path: &str) -> Reply {
        send_as(
            &self.app,
            Some(self.device),
            Method::GET,
            CORE,
            path,
            &[],
            "",
        )
        .await
    }

    fn seed_bot(&self, name: &str, user_id: &str, keeper: BotKeeper) {
        self.home.manager.bot_records_for_test(
            &PluginId::parse("app.whoami").unwrap(),
            &[(name, user_id, keeper, "test-token")],
        );
    }

    fn set_home_room(&self, bot: &str, room: &str) {
        let path = self.home.data_dir.join("plugins/bots/app.whoami.json");
        let mut records: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        records["bots"][bot]["home_room"] = room.into();
        std::fs::write(path, serde_json::to_vec(&records).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn own_ready_identity_survives_matching_reservation_and_role_closure() {
    let reference = bot("user-mia");
    let rig = Rig::start(true, Some(reference.clone()), Some(reference.clone())).await;
    rig.seed_bot(
        &reference.bot,
        "@mia-bot:test",
        BotKeeper::User {
            username: "mia".into(),
        },
    );
    let reply = rig.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["requester_user_id"], "@mia:test");
    assert_eq!(reply.json()["result"]["state"], "ready");
    assert_eq!(reply.json()["result"]["bot"]["plugin_id"], "app.whoami");
    assert_eq!(reply.json()["result"]["bot"]["bot"], "user-mia");
    assert_eq!(reply.json()["result"]["user_id"], "@mia-bot:test");
    assert_eq!(reply.json()["result"]["home_room"], "!user-mia:test");
    assert!(reply.json()["result"]["room_problem"].is_null());
    assert!(!reply.text().contains("test-token"));
    let response: OwnPrivateBotResponse = serde_json::from_str(reply.text()).unwrap();
    assert_eq!(response.requester_user_id, "@mia:test");
    let OwnPrivateBotResult::Ready(identity) = response.result else {
        panic!("expected verified private Bot");
    };
    assert_eq!(identity.bot, reference);
    assert_eq!(identity.user_id, "@mia-bot:test");

    rig.home.members.set_role("mia", "admin").unwrap();
    let role_closed = rig.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(role_closed.status, StatusCode::OK, "{}", role_closed.text());
    assert_eq!(role_closed.json(), reply.json());
}

/// 不要私人 Bot 的人（角色里没有，或者自己没选）是“不适用”，和“有私人 Bot 但还没建好”分开：
/// 前者用默认 Bot，后者等建好。
#[tokio::test]
async fn no_private_bot_is_not_applicable_and_pending_is_distinct() {
    let none = Rig::start(false, None, None).await;
    let reply = none.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["requester_user_id"], "@mia:test");
    assert_eq!(reply.json()["result"]["state"], "not_applicable");

    let no_matrix = send_as(
        &none.home.app,
        Some(none.device),
        Method::GET,
        CORE,
        MY_PRIVATE_BOT_PATH,
        &[],
        "",
    )
    .await;
    assert_eq!(no_matrix.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(no_matrix.error_kind(), Some("matrix_unavailable"));
    none.home.members.set_role("mia", "missing-role").unwrap();
    let no_role = none.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(no_role.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(no_role.error_kind(), Some("member_role_unavailable"));

    let pending = Rig::start(true, None, None).await;
    let reply = pending.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("private_bot_pending"));
    pending
        .home
        .members
        .reserve_private_bot("mia", &bot("user-mia"))
        .unwrap();
    let reserved = pending.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(reserved.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reserved.error_kind(), Some("private_bot_pending"));
}

#[tokio::test]
async fn conflicting_missing_or_corrupt_facts_never_report_absence() {
    let conflict = Rig::start(true, Some(bot("user-mia")), Some(bot("other"))).await;
    let reply = conflict.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_kind(), Some("private_bot_conflict"));

    let missing = Rig::start(true, Some(bot("user-mia")), Some(bot("user-mia"))).await;
    let reply = missing.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("private_bot_record_missing"));
    missing.seed_bot(
        "user-mia",
        "@mia-bot:test",
        BotKeeper::User {
            username: "owner".into(),
        },
    );
    let wrong_keeper = missing.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(wrong_keeper.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        wrong_keeper.error_kind(),
        Some("private_bot_identity_mismatch")
    );
    std::fs::write(
        missing.home.data_dir.join("plugins/bots/app.whoami.json"),
        "{",
    )
    .unwrap();
    let corrupt = missing.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(corrupt.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(corrupt.error_kind(), Some("bot_record_corrupt"));

    let path = missing.home.data_dir.join("plugins/bots/app.whoami.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let io = missing.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(io.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(io.error_kind(), Some("bot_record_unavailable"));
}

#[tokio::test]
async fn original_authentication_and_origin_are_preserved() {
    let rig = Rig::start(true, Some(bot("user-mia")), Some(bot("user-mia"))).await;
    rig.seed_bot(
        "user-mia",
        "@mia-bot:test",
        BotKeeper::User {
            username: "mia".into(),
        },
    );
    let cross_site = send_as(
        &rig.app,
        Some(rig.device),
        Method::GET,
        CORE,
        MY_PRIVATE_BOT_PATH,
        &[("origin", "https://another.example")],
        "",
    )
    .await;
    assert_eq!(cross_site.status, StatusCode::FORBIDDEN);
    let token = bearer();
    let owner = send_as(
        &rig.app,
        Some(rig.device),
        Method::GET,
        CORE,
        MY_PRIVATE_BOT_PATH,
        &[("authorization", &token)],
        "",
    )
    .await;
    assert_eq!(owner.status, StatusCode::OK, "{}", owner.text());
    assert_eq!(owner.json()["requester_user_id"], "@owner:test");
    assert_eq!(owner.json()["result"]["state"], "not_applicable");
    assert!(!owner.text().contains("@mia-bot:test"));
    let owner_without_matrix = send_as(
        &rig.home.app,
        None,
        Method::GET,
        CORE,
        MY_PRIVATE_BOT_PATH,
        &[("authorization", &token)],
        "",
    )
    .await;
    assert_eq!(owner_without_matrix.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        owner_without_matrix.error_kind(),
        Some("matrix_unavailable")
    );
    let queried = rig
        .read(&format!("{MY_PRIVATE_BOT_PATH}?username=someone-else"))
        .await;
    assert_eq!(queried.status, StatusCode::OK, "{}", queried.text());
    assert_eq!(queried.json()["requester_user_id"], "@mia:test");
    assert_eq!(queried.json()["result"]["user_id"], "@mia-bot:test");
}

#[tokio::test]
async fn invalid_or_unknown_home_room_keeps_verified_identity_without_a_target() {
    let reference = bot("user-mia");
    let rig = Rig::start(true, Some(reference.clone()), Some(reference)).await;
    rig.seed_bot(
        "user-mia",
        "@mia-bot:test",
        BotKeeper::User {
            username: "mia".into(),
        },
    );

    rig.set_home_room("user-mia", "not-a-matrix-room");
    let invalid = rig.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(invalid.status, StatusCode::OK, "{}", invalid.text());
    assert_eq!(invalid.json()["result"]["state"], "ready");
    assert_eq!(invalid.json()["result"]["user_id"], "@mia-bot:test");
    assert!(invalid.json()["result"]["home_room"].is_null());
    assert_eq!(invalid.json()["result"]["room_problem"], "invalid");
    assert!(!invalid.text().contains("not-a-matrix-room"));

    rig.set_home_room("user-mia", "");
    let unknown = rig.read(MY_PRIVATE_BOT_PATH).await;
    assert_eq!(unknown.status, StatusCode::OK, "{}", unknown.text());
    assert_eq!(unknown.json()["result"]["user_id"], "@mia-bot:test");
    assert!(unknown.json()["result"]["home_room"].is_null());
    assert_eq!(unknown.json()["result"]["room_problem"], "missing");
}

#[test]
fn wire_response_requires_the_requester_and_preserves_unknown_result() {
    let null = serde_json::Value::Null;
    assert!(serde_json::from_value::<OwnPrivateBotResponse>(null).is_err());
    let unbound = serde_json::json!({"result": {"state": "absent"}});
    assert!(serde_json::from_value::<OwnPrivateBotResponse>(unbound).is_err());
    let future = serde_json::json!({
        "requester_user_id": "@mia:test",
        "result": {"state": "future_state"}
    });
    let response: OwnPrivateBotResponse = serde_json::from_value(future).unwrap();
    assert_eq!(response.result, OwnPrivateBotResult::Other);
}
