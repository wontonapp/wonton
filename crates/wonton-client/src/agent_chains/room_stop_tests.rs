//! 行为测试：给定链列表和停的请求走到哪，断言时间线的停止控件。

use serde_json::json;
use wonton_protocol::{AgentChainsViewModel, CoreError, RoomAgentStop, TimelineTarget};

use super::{Attempt, Attempts, derive, key};

const ROOM: &str = "!r:home.example";

fn main() -> TimelineTarget {
    TimelineTarget {
        room_id: ROOM.into(),
        thread_root: None,
    }
}

fn row(
    chain: &str,
    room: &str,
    thread: Option<&str>,
    status: &str,
    live: bool,
) -> serde_json::Value {
    json!({
        "chain": chain,
        "user": "@alex:home.example",
        "origin": {"room_id": room, "thread_id": thread},
        "bots": [{"bot": {"plugin_id": "app.hermes", "bot": "owner"}, "layer": 1}],
        "rounds": 1,
        "status": status,
        "can_stop": true,
        "live": live,
    })
}

fn view(rows: Vec<serde_json::Value>) -> AgentChainsViewModel {
    serde_json::from_value(json!({"chains": rows, "revision": 1})).unwrap()
}

fn attempts(attempt: Attempt) -> Attempts {
    Attempts::from([(key(&main()), attempt)])
}

fn forbidden() -> CoreError {
    CoreError::Forbidden {
        reason: "not allowed".into(),
    }
}

fn offline() -> CoreError {
    CoreError::TransportUnavailable {
        reason: "offline".into(),
    }
}

/// 只有从这条时间线开始、还有轮次在跑的链才给停止控件：别的房间、旁聊、做完的一轮、停了的链都不算。
#[test]
fn the_control_shows_only_for_a_live_chain_of_this_timeline() {
    let none = Attempts::new();
    let running = view(vec![row("c-1", ROOM, None, "running", true)]);
    assert_eq!(derive(&running, &none, &main()), RoomAgentStop::Running);
    let waiting_on_limit = view(vec![row("c-1", ROOM, None, "limit_reached", true)]);
    assert_eq!(
        derive(&waiting_on_limit, &none, &main()),
        RoomAgentStop::Running
    );

    for idle in [
        view(vec![]),
        view(vec![row("c-1", ROOM, None, "running", false)]),
        view(vec![row("c-1", ROOM, None, "stopped", true)]),
        view(vec![row(
            "c-1",
            "!other:home.example",
            None,
            "running",
            true,
        )]),
        view(vec![row("c-1", ROOM, Some("$root"), "running", true)]),
    ] {
        assert_eq!(derive(&idle, &none, &main()), RoomAgentStop::Idle);
    }

    let side_chat = TimelineTarget {
        room_id: ROOM.into(),
        thread_root: Some("$root".into()),
    };
    let in_thread = view(vec![row("c-1", ROOM, Some("$root"), "running", true)]);
    assert_eq!(
        derive(&in_thread, &none, &side_chat),
        RoomAgentStop::Running
    );
}

/// 比客户端旧的核心不报 `live`：不知道有没有轮次在跑，不给停止控件。
#[test]
fn a_core_that_does_not_report_live_rounds_gets_no_control() {
    let mut old = row("c-1", ROOM, None, "running", true);
    old.as_object_mut().unwrap().remove("live");
    assert_eq!(
        derive(&view(vec![old]), &Attempts::new(), &main()),
        RoomAgentStop::Idle
    );
}

/// 请求在路上一直是“在停”；失败和不确定只在链还在跑时显示，链没了就没什么可重试的；
/// 停成了显示“停了”，同一条时间线又开一轮时让位给新的停止控件。
#[test]
fn the_attempt_and_the_list_together_decide_the_control() {
    let live = view(vec![row("c-1", ROOM, None, "running", true)]);
    let gone = view(vec![]);

    for list in [&live, &gone] {
        assert_eq!(
            derive(list, &attempts(Attempt::Stopping), &main()),
            RoomAgentStop::Stopping
        );
    }
    assert_eq!(
        derive(&live, &attempts(Attempt::Failed(forbidden())), &main()),
        RoomAgentStop::Failed { error: forbidden() }
    );
    assert_eq!(
        derive(&live, &attempts(Attempt::Uncertain(offline())), &main()),
        RoomAgentStop::Uncertain { error: offline() }
    );
    for settled in [Attempt::Failed(forbidden()), Attempt::Uncertain(offline())] {
        assert_eq!(
            derive(&gone, &attempts(settled), &main()),
            RoomAgentStop::Idle
        );
    }
    assert_eq!(
        derive(&gone, &attempts(Attempt::Stopped), &main()),
        RoomAgentStop::Stopped
    );
    assert_eq!(
        derive(&live, &attempts(Attempt::Stopped), &main()),
        RoomAgentStop::Running
    );
}
