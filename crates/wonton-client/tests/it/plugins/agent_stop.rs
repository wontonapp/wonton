//! 聊天里的停止控件：只经导出给壳的接口，假核心给链列表和停链的回应，断言控件依次变成什么。

use axum::http::Method;
use serde_json::json;
use wonton_client::{Action, AgentStopSubscription, CoreError};
use wonton_protocol::{AgentAction, RoomAgentStop, TimelineTarget};

use super::rig::{Canned, Rig, WAIT};

const LIST: &str = "/_wonton/api/agent/chains";
const STOP: &str = "/_wonton/api/agent/chains/c-1/stop";

fn target() -> TimelineTarget {
    TimelineTarget {
        room_id: "!r:home.example".into(),
        thread_root: None,
    }
}

fn row(status: &str, live: bool) -> serde_json::Value {
    json!({
        "chain": "c-1",
        "user": "@alex:home.example",
        "origin": {"room_id": "!r:home.example", "thread_id": null},
        "bots": [{"bot": {"plugin_id": "app.hermes", "bot": "owner"}, "layer": 1}],
        "rounds": 1,
        "status": status,
        "can_stop": true,
        "live": live,
    })
}

fn stop() -> Action {
    Action::Agent {
        action: AgentAction::StopTurns { target: target() },
    }
}

async fn next(subscription: &AgentStopSubscription) -> RoomAgentStop {
    tokio::time::timeout(WAIT, subscription.next())
        .await
        .expect("停止控件没有等到下一个样子")
        .expect("停止控件的订阅提前结束")
}

/// 连上家里、列表里有一条在跑的链，控件到“能停”为止。
async fn rig_with_a_live_turn() -> (Rig, std::sync::Arc<AgentStopSubscription>) {
    let rig = Rig::new().await;
    rig.core.answer(
        Method::GET,
        LIST,
        Canned(
            200,
            json!({"chains": [row("running", true)], "revision": 1}).to_string(),
        ),
    );
    rig.connected().await;
    let subscription = rig.app.subscribe_agent_stop(target());
    loop {
        if next(&subscription).await == RoomAgentStop::Running {
            break;
        }
    }
    (rig, subscription)
}

/// 点停止：请求在路上是“在停”，核心确认后是“停了”，过一会儿控件自己收起来；链只停一次。
#[tokio::test]
async fn stopping_a_live_turn_goes_through_stopping_to_stopped_then_clears() {
    let (rig, subscription) = rig_with_a_live_turn().await;
    rig.core.hold(Method::POST, STOP);
    rig.core.answer(
        Method::POST,
        STOP,
        Canned(200, row("stopped", false).to_string()),
    );

    rig.app.dispatch(stop()).unwrap();
    assert_eq!(next(&subscription).await, RoomAgentStop::Stopping);
    // 在停的时候再点不重复发。
    rig.app.dispatch(stop()).unwrap();
    rig.core.release(Method::POST, STOP);
    assert_eq!(next(&subscription).await, RoomAgentStop::Stopped);
    assert_eq!(next(&subscription).await, RoomAgentStop::Idle);
    assert_eq!(rig.core.seen(&Method::POST, STOP).len(), 1);
}

/// 核心不让停：控件带着原因类别回到能再点的样子，链还在跑；再点一次重新发请求。
#[tokio::test]
async fn a_refused_stop_is_shown_with_its_category_and_can_be_retried() {
    let (rig, subscription) = rig_with_a_live_turn().await;
    rig.core.answer(
        Method::POST,
        STOP,
        Canned(403, json!({"error": {"message": "not yours"}}).to_string()),
    );
    rig.core.answer(
        Method::POST,
        STOP,
        Canned(200, row("stopped", false).to_string()),
    );

    rig.app.dispatch(stop()).unwrap();
    assert_eq!(next(&subscription).await, RoomAgentStop::Stopping);
    assert!(matches!(
        next(&subscription).await,
        RoomAgentStop::Failed {
            error: CoreError::Forbidden { .. }
        }
    ));

    rig.app.dispatch(stop()).unwrap();
    assert_eq!(next(&subscription).await, RoomAgentStop::Stopping);
    assert_eq!(next(&subscription).await, RoomAgentStop::Stopped);
}

/// 请求没送到家里：停没停不知道，按不确定显示，不说成停了，也不说成没停。
#[tokio::test]
async fn a_stop_that_never_reaches_home_is_uncertain() {
    let (rig, subscription) = rig_with_a_live_turn().await;
    rig.core
        .answer(Method::POST, STOP, Canned(502, String::new()));

    rig.app.dispatch(stop()).unwrap();
    assert_eq!(next(&subscription).await, RoomAgentStop::Stopping);
    assert!(matches!(
        next(&subscription).await,
        RoomAgentStop::Uncertain {
            error: CoreError::TransportUnavailable { .. }
        }
    ));
}

/// 没有在跑的轮次时点停止什么都不发；没连上家里时也没有控件可点。
#[tokio::test]
async fn nothing_is_sent_without_a_live_turn() {
    let rig = Rig::new().await;
    let subscription = rig.app.subscribe_agent_stop(target());
    assert_eq!(next(&subscription).await, RoomAgentStop::Idle);
    rig.app.dispatch(stop()).unwrap();
    rig.core.answer(
        Method::GET,
        LIST,
        Canned(
            200,
            json!({"chains": [row("running", false)], "revision": 1}).to_string(),
        ),
    );
    rig.connected().await;
    tokio::time::timeout(WAIT, async {
        while rig.app.agent_chains().chains.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("链列表没有取到");
    rig.app.dispatch(stop()).unwrap();
    assert!(rig.core.seen(&Method::POST, STOP).is_empty());
}
