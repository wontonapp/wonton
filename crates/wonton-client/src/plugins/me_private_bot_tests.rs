use std::sync::{Arc, Mutex};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::*;
use crate::plugins::api::ServerVersion;

type Reply = Arc<Mutex<(StatusCode, Value)>>;

async fn reply(State(reply): State<Reply>) -> (StatusCode, Json<Value>) {
    let (status, body) = reply.lock().unwrap().clone();
    (status, Json(body))
}

async fn server() -> (CoreApi, Reply, JoinHandle<()>) {
    let reply = Arc::new(Mutex::new((StatusCode::OK, Value::Null)));
    let router = Router::new()
        .route(MY_PRIVATE_BOT_PATH, get(self::reply))
        .with_state(reply.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let api = CoreApi {
        base: format!("http://{address}").parse().unwrap(),
        client: reqwest::Client::new(),
        languages: String::new(),
        version: ServerVersion::default(),
    };
    (api, reply, server)
}

fn ready() -> Value {
    json!({
        "requester_user_id": "@mia:test",
        "result": {
            "state": "ready",
            "bot": {"plugin_id": "app.whoami", "bot": "mia-bot"},
            "user_id": "@mia-bot:test",
            "home_room": "!main:test",
            "room_problem": null
        }
    })
}

fn respond(reply: &Reply, status: StatusCode, body: Value) {
    *reply.lock().unwrap() = (status, body);
}

#[tokio::test]
async fn response_is_bound_to_matrix_session_and_validated() {
    let (api, reply, server) = server().await;
    let session_user = <&UserId>::try_from("@mia:test").unwrap();
    respond(&reply, StatusCode::OK, ready());
    let Ok(view) = fetch_own_private_bot(&api, session_user).await else {
        panic!("valid identity was not returned");
    };
    let OwnPrivateBotView::Ready {
        bot,
        user_id,
        home_room,
        room_problem,
    } = view
    else {
        panic!("ready identity was not returned");
    };
    assert_eq!(
        bot,
        BotRef {
            plugin_id: "app.whoami".into(),
            bot: "mia-bot".into(),
        }
    );
    assert_eq!(user_id, "@mia-bot:test");
    assert_eq!(home_room.as_deref(), Some("!main:test"));
    assert_eq!(room_problem, None);

    for body in [json!(null), json!({"result": {"state": "absent"}})] {
        respond(&reply, StatusCode::OK, body);
        assert!(matches!(
            fetch_own_private_bot(&api, session_user).await,
            Err(OwnBotProblem {
                category: OwnBotFailure::Protocol,
                ..
            })
        ));
    }

    for (value, expected_kind) in [
        (json!("@other:test"), "requester_mismatch"),
        (json!("not-a-user-id"), "identity_invalid"),
    ] {
        let mut body = ready();
        body["requester_user_id"] = value;
        respond(&reply, StatusCode::OK, body);
        let Err(problem) = fetch_own_private_bot(&api, session_user).await else {
            panic!("invalid requester was accepted");
        };
        assert_eq!(problem.category, OwnBotFailure::Protocol);
        assert_eq!(problem.kind.as_deref(), Some(expected_kind));
        assert_eq!(problem.status, None);
        assert!(!problem.detail.contains("@other:test"));
    }
    server.abort();
}

#[tokio::test]
async fn personal_bot_view_preserves_identity_and_unknown_room() {
    let (api, reply, server) = server().await;
    let session_user = <&UserId>::try_from("@mia:test").unwrap();
    for (field, value, expected_kind) in [
        ("user_id", json!("@bot:elsewhere"), Some("identity_invalid")),
        ("user_id", json!("@mia:test"), Some("identity_invalid")),
        ("user_id", json!("invalid"), Some("identity_invalid")),
        (
            "home_room",
            json!("#not-a-room:test"),
            Some("identity_invalid"),
        ),
        (
            "bot",
            json!({"plugin_id": "../other", "bot": "mia-bot"}),
            Some("identity_invalid"),
        ),
        (
            "bot",
            json!({"plugin_id": "app.whoami", "bot": "bad/name"}),
            Some("identity_invalid"),
        ),
        ("room_problem", json!("invalid"), None),
    ] {
        let mut body = ready();
        body["result"][field] = value;
        respond(&reply, StatusCode::OK, body);
        let Err(problem) = fetch_own_private_bot(&api, session_user).await else {
            panic!("invalid identity was accepted");
        };
        assert_eq!(problem.category, OwnBotFailure::Protocol);
        assert_eq!(problem.kind.as_deref(), expected_kind);
        assert_eq!(problem.status, None);
    }
    for (state, expected) in [
        ("not_applicable", OwnPrivateBotView::NotApplicable),
        ("absent", OwnPrivateBotView::Absent),
        ("future_state", OwnPrivateBotView::Other),
    ] {
        respond(
            &reply,
            StatusCode::OK,
            json!({
                "requester_user_id": "@mia:test", "result": {"state": state}
            }),
        );
        let Ok(view) = fetch_own_private_bot(&api, session_user).await else {
            panic!("valid state was not returned");
        };
        assert_eq!(view, expected);
    }
    let mut unknown_room = ready();
    unknown_room["result"]["home_room"] = Value::Null;
    unknown_room["result"]["room_problem"] = json!("future_problem");
    respond(&reply, StatusCode::OK, unknown_room);
    let Ok(view) = fetch_own_private_bot(&api, session_user).await else {
        panic!("unknown room problem was rejected");
    };
    assert!(matches!(
        view,
        OwnPrivateBotView::Ready {
            home_room: None,
            room_problem: Some(wonton_protocol::PrivateBotRoomProblem::Other),
            ..
        }
    ));
    respond(
        &reply,
        StatusCode::OK,
        json!({
            "requester_user_id": "@other:test", "result": {"state": "absent"}
        }),
    );
    assert!(matches!(
        fetch_own_private_bot(&api, session_user).await,
        Err(OwnBotProblem {
            category: OwnBotFailure::Protocol,
            ..
        })
    ));
    server.abort();
}

#[tokio::test]
async fn http_errors_preserve_status_and_kind_without_becoming_absent() {
    let (api, reply, server) = server().await;
    let session_user = <&UserId>::try_from("@mia:test").unwrap();
    for (status, kind, category) in [
        (
            StatusCode::NOT_FOUND,
            Some("role_not_found"),
            OwnBotFailure::NotReady,
        ),
        (
            StatusCode::NOT_FOUND,
            None,
            OwnBotFailure::CapabilityUnknown,
        ),
        (
            StatusCode::NOT_FOUND,
            Some("future_kind"),
            OwnBotFailure::CapabilityUnknown,
        ),
        (
            StatusCode::UNAUTHORIZED,
            Some("unauthenticated"),
            OwnBotFailure::Unauthorized,
        ),
        (
            StatusCode::FORBIDDEN,
            Some("permission_required"),
            OwnBotFailure::PermissionDenied,
        ),
        (
            StatusCode::FORBIDDEN,
            None,
            OwnBotFailure::TransportUnavailable,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Some("private_bot_pending"),
            OwnBotFailure::NotReady,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Some("future_upstream"),
            OwnBotFailure::Upstream,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Some("bot_record_unavailable"),
            OwnBotFailure::StorageUnavailable,
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Some("bot_record_corrupt"),
            OwnBotFailure::StorageCorrupt,
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Some("private_bot_identity_mismatch"),
            OwnBotFailure::RecordInconsistent,
        ),
    ] {
        let body = kind.map_or_else(
            || json!({}),
            |kind| json!({"error": {"kind": kind, "message": "upstream detail"}}),
        );
        respond(&reply, status, body);
        let Err(problem) = fetch_own_private_bot(&api, session_user).await else {
            panic!("HTTP error was not preserved");
        };
        assert_eq!(problem.category, category);
        assert_eq!(problem.status, Some(status.as_u16()));
        assert_eq!(problem.kind.as_deref(), kind);
        assert!(!problem.detail.contains("upstream detail"));
    }
    server.abort();
}

#[tokio::test]
async fn unavailable_transport_is_not_treated_as_an_absent_bot() {
    let api = CoreApi {
        base: "http://127.0.0.1:0".parse().unwrap(),
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
        languages: String::new(),
        version: ServerVersion::default(),
    };
    let session_user = <&UserId>::try_from("@mia:test").unwrap();
    let Err(problem) = fetch_own_private_bot(&api, session_user).await else {
        panic!("unavailable transport returned an identity");
    };
    assert_eq!(problem.category, OwnBotFailure::TransportUnavailable);
    assert_eq!(problem.status, None);
    assert_eq!(problem.kind, None);
    assert!(!problem.detail.contains("@mia:test"));
}
