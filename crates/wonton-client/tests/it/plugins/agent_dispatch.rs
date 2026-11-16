use axum::http::Method;
use serde_json::json;
use wonton_client::{Action, CoreError};
use wonton_protocol::{AgentAction, AgentChainStatus};

use super::rig::{Canned, Rig, WAIT};

#[tokio::test]
async fn stop_chain_dispatch_updates_the_subscribed_list() {
    let rig = Rig::new().await;
    let stop = || Action::Agent {
        action: AgentAction::StopChain {
            chain: "c-1".into(),
        },
    };
    assert!(matches!(
        rig.app.dispatch(stop()),
        Err(CoreError::TransportUnavailable { .. })
    ));

    let row = |status| {
        json!({
            "chain": "c-1",
            "user": "@alex:home.example",
            "origin": {"room_id": "!r:home.example", "thread_id": null},
            "bots": [{"bot": {"plugin_id": "app.orchestrator", "bot": "owner"}, "layer": 1}],
            "rounds": 2,
            "spend": {"type": "unknown"},
            "status": status,
            "can_stop": true,
        })
    };
    let list_path = "/_wonton/api/agent/chains";
    let stop_path = "/_wonton/api/agent/chains/c-1/stop";
    rig.core.answer(
        Method::GET,
        list_path,
        Canned(
            200,
            json!({
                "chains": [row("running")], "revision": 1,
            })
            .to_string(),
        ),
    );
    rig.core.hold(Method::GET, list_path);
    rig.core.answer(
        Method::POST,
        stop_path,
        Canned(200, row("stopped").to_string()),
    );
    rig.connected().await;
    let subscription = rig.app.subscribe_agent_chains();
    rig.core.release(Method::GET, list_path);
    tokio::time::timeout(WAIT, async {
        while let Some(view) = subscription.next().await {
            if view
                .chains
                .iter()
                .any(|chain| chain.status == AgentChainStatus::Running)
            {
                return;
            }
        }
        panic!("链列表订阅提前结束");
    })
    .await
    .expect("链列表没有显示运行中的链");

    rig.app.dispatch(stop()).unwrap();
    tokio::time::timeout(WAIT, async {
        while let Some(view) = subscription.next().await {
            if view
                .chains
                .iter()
                .any(|chain| chain.status == AgentChainStatus::Stopped)
            {
                return;
            }
        }
        panic!("链列表订阅提前结束");
    })
    .await
    .expect("停链后列表没有更新");
    assert_eq!(rig.core.seen(&Method::POST, stop_path).len(), 1);
}

#[tokio::test]
async fn rejected_agent_action_is_published_and_cleared_on_retry() {
    let rig = Rig::new().await;
    let path = "/_wonton/api/agent/chains/c-1/stop";
    rig.core.answer(
        Method::POST,
        path,
        Canned(
            403,
            json!({"error": {"message": "not allowed"}}).to_string(),
        ),
    );
    rig.core.hold(Method::GET, "/_wonton/api/agent/chains");
    rig.connected().await;
    let subscription = rig.app.subscribe_agent_chains();
    let action = AgentAction::StopChain {
        chain: "c-1".into(),
    };
    rig.app
        .dispatch(Action::Agent {
            action: action.clone(),
        })
        .unwrap();

    let failure = tokio::time::timeout(WAIT, async {
        while let Some(view) = subscription.next().await {
            if let Some(failure) = view.action_failure {
                return failure;
            }
        }
        panic!("链列表订阅提前结束");
    })
    .await
    .expect("动作失败没有交给订阅者");
    assert_eq!(failure.action, action);
    assert!(matches!(failure.error, CoreError::Forbidden { .. }));

    rig.core.hold(Method::POST, path);
    rig.app.dispatch(Action::Agent { action }).unwrap();
    assert!(rig.app.agent_chains().action_failure.is_none());
    rig.core.release(Method::POST, path);
}
