//! 模式白名单：桥只发配置的白名单模式；Agent 在 `availableModes` 里报出危险档位也不跟。

use super::*;

#[tokio::test]
async fn the_bridge_never_sends_dangerous_modes() {
    let harness = Harness::in_scenario(Scenario::DangerousModes, |_| {}).await;
    harness.send("$m1", "帮我看看");
    harness.wait_prompts(1).await;
    let requests = harness.agent.requests().await;
    let set_modes: Vec<_> = requests
        .iter()
        .filter(|request| request["method"] == "session/set_mode")
        .collect();
    assert_eq!(set_modes.len(), 1, "{requests:?}");
    assert_eq!(set_modes[0]["params"]["modeId"], "plan");
    assert!(
        requests
            .iter()
            .all(|request| request["params"]["modeId"] != "bypassPermissions"),
        "Agent 报了 bypassPermissions，桥不能跟着发：{requests:?}"
    );
    harness.respond("session-1", "好了").await;
    harness.finish().await;
}
