//! Agent 进度状态消息的行为测试（ADR-0082 第 1 节完成标准）：一轮一条、编辑合并、
//! 终态类别、话题挂靠、重启恢复。报文样例见 `fixtures/run-status/`。

use serde_json::{Value, json};

use super::*;
use crate::bridge::test_support::{Effect, wait_for, wait_until};

/// 一条 `tool_call` 更新：类别自选，标题带命令和路径（断言它们不进状态消息）。
fn tool_call(id: &str, kind: &str) -> Value {
    json!({
        "sessionUpdate": "tool_call",
        "toolCallId": id,
        "title": "rm -rf /tmp/secret && cat /etc/shadow",
        "kind": kind,
        "locations": [{"path": "/tmp/secret"}],
        "rawInput": {"command": "rm -rf /tmp/secret"}
    })
}

/// 一条只更新标题的 `tool_call_update`（不该动状态）。
fn tool_call_update(id: &str) -> Value {
    json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": id,
        "title": "改 /tmp/secret",
        "status": "completed"
    })
}

async fn start_turn(harness: &Harness, request: &str) -> String {
    harness.send(request, "改登录");
    let prompts = harness.wait_prompts(1).await;
    prompts[0].0.clone()
}

/// 状态对象里 `org.fluxchat.agent` 的内容。
fn agent_status(content: &Value) -> &Value {
    &content["org.fluxchat.agent"]
}

#[tokio::test]
async fn a_turn_reports_working_then_the_answer_then_done() {
    let harness = Harness::start().await;
    let session = start_turn(&harness, "$req1").await;
    wait_until(|| harness.room.run_statuses().len() == 1).await;
    let (event_id, thread, content) = harness.room.run_statuses()[0].clone();
    assert_eq!(event_id, "$status1");
    assert_eq!(thread, None);
    assert_eq!(content["body"], json!("在想"));
    assert_eq!(
        agent_status(&content),
        &json!({"v": 1, "run": "$req1", "state": "working", "step": "在想", "steps": 0})
    );

    harness
        .agent
        .raw_update(&session, tool_call("call-1", "read"))
        .await;
    harness
        .agent
        .raw_update(&session, tool_call("call-2", "edit"))
        .await;
    harness
        .agent
        .raw_update(&session, tool_call_update("call-2"))
        .await;
    harness.respond(&session, "登录改好了").await;

    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .last()
            .is_some_and(|edit| agent_status(edit)["state"] == json!("done"))
    })
    .await;
    let edits = harness.room.run_edits("$status1");
    let done_edit = edits.last().unwrap();
    let effects = harness.room.effects();
    let answer = effects
        .iter()
        .position(|effect| matches!(effect, Effect::MainText(text) if text == "登录改好了"))
        .expect("答案发出去了");
    let done = effects
        .iter()
        .position(|effect| matches!(effect, Effect::ReplaceRunStatus { event_id, .. } if event_id == "$status1"))
        .expect("状态编辑成了 done");
    assert!(answer < done, "先发答案，再编辑状态");
    assert_eq!(
        done_edit["m.relates_to"],
        json!({"rel_type": "m.replace", "event_id": "$status1"})
    );
    assert_eq!(
        agent_status(done_edit),
        &json!({
            "v": 1, "run": "$req1", "state": "done",
            "steps": 2, "result": "完成，用了 2 步"
        })
    );
    harness.finish().await;
}

#[tokio::test]
async fn a_permission_request_edits_to_waiting_and_back_to_working() {
    let harness = Harness::start().await;
    let session = start_turn(&harness, "$req1").await;
    harness.agent.ask_permission(&session, "src/a.rs").await;
    wait_until(|| harness.cards.pending() == 1).await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .iter()
            .any(|edit| agent_status(edit)["state"] == json!("waiting"))
    })
    .await;
    let waiting = harness
        .room
        .run_edits("$status1")
        .into_iter()
        .find(|edit| agent_status(edit)["state"] == json!("waiting"))
        .unwrap();
    assert_eq!(waiting["body"], json!("等你确认"));

    harness.cards.answer("allow_once");
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .iter()
            .any(|edit| agent_status(edit)["state"] == json!("working"))
    })
    .await;
    harness.respond(&session, "改好了").await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .last()
            .is_some_and(|edit| agent_status(edit)["state"] == json!("done"))
    })
    .await;
    harness.finish().await;
}

#[tokio::test]
async fn stop_reasons_end_the_run_with_their_own_categories() {
    for (stop_reason, result) in [
        ("cancelled", json!("打断了")),
        ("max_tokens", json!("失败：回复达到输出长度上限")),
        ("max_turn_requests", json!("失败：达到轮次上限")),
        ("refusal", json!("失败：Agent 拒绝了请求")),
    ] {
        let harness = Harness::start().await;
        let session = start_turn(&harness, "$req1").await;
        harness
            .agent
            .raw_update(&session, tool_call("call-1", "read"))
            .await;
        harness
            .agent
            .respond_prompt_as(0, "没说完", stop_reason)
            .await;
        let expected = result;
        wait_until(|| {
            harness
                .room
                .run_edits("$status1")
                .last()
                .is_some_and(|edit| agent_status(edit)["result"] == expected)
        })
        .await;
        let edits = harness.room.run_edits("$status1");
        let edit = edits.last().unwrap();
        assert_eq!(agent_status(edit)["state"], json!("failed"));
        assert_eq!(
            edit["app.wonton.end"],
            if stop_reason == "cancelled" {
                json!("cancelled")
            } else {
                Value::Null
            }
        );
        harness.finish().await;
    }
}

#[tokio::test]
async fn a_connection_drop_while_a_tool_runs_is_uncertain() {
    let harness = Harness::start().await;
    let session = start_turn(&harness, "$req1").await;
    harness
        .agent
        .raw_update(&session, tool_call("call-1", "execute"))
        .await;
    // 等回答的编辑不受 3 秒合并限制：它带着 steps=1 出现，说明工具调用已被桥消化，
    // 这时再断线，稳拿“不确定”而不是竞态。
    harness.agent.ask_permission(&session, "src/a.rs").await;
    wait_until(|| {
        harness.room.run_edits("$status1").iter().any(|edit| {
            agent_status(edit)["state"] == json!("waiting")
                && agent_status(edit)["steps"] == json!(1)
        })
    })
    .await;
    harness.agent.drop_connection().await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .last()
            .is_some_and(|edit| agent_status(edit)["state"] == json!("failed"))
    })
    .await;
    let edits = harness.room.run_edits("$status1");
    let edit = edits.last().unwrap();
    assert_eq!(edit["app.wonton.end"], json!("uncertain"));
    assert_eq!(agent_status(edit)["result"], json!("结果不确定"));
    harness.finish().await;
}

#[tokio::test]
async fn a_connection_drop_without_tools_reports_the_transport_error() {
    let harness = Harness::start().await;
    let _session = start_turn(&harness, "$req1").await;
    harness.agent.drop_connection().await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .last()
            .is_some_and(|edit| agent_status(edit)["state"] == json!("failed"))
    })
    .await;
    let edits = harness.room.run_edits("$status1");
    let edit = edits.last().unwrap();
    assert_eq!(edit["app.wonton.end"], Value::Null);
    assert_eq!(
        agent_status(edit)["result"],
        json!(format_error(
            &Error::TransportUnavailable(Detail::msg("acp-connection-closed")),
            crate::l10n::zh(),
        ))
    );
    harness.finish().await;
}

#[tokio::test]
async fn fifty_step_only_tool_calls_in_a_second_merge_into_at_most_two_edits() {
    let harness = Harness::start().await;
    let session = start_turn(&harness, "$req1").await;
    for index in 0..50 {
        let id = format!("call-{index}");
        harness
            .agent
            .raw_update(&session, tool_call(&id, "read"))
            .await;
    }
    harness.respond(&session, "好了").await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .last()
            .is_some_and(|edit| agent_status(edit)["state"] == json!("done"))
    })
    .await;
    let edits = harness.room.run_edits("$status1");
    assert!(
        edits.len() <= 2,
        "1 秒内的步骤变化只该合并出最多一次中段编辑，实际 {} 次",
        edits.len()
    );
    assert_eq!(
        agent_status(edits.last().unwrap())["steps"],
        json!(50),
        "最终状态对"
    );
    harness.finish().await;
}

#[tokio::test]
async fn topic_turns_hang_the_status_on_the_topic_and_main_turns_do_not() {
    let harness = Harness::start().await;
    harness.send_in_thread("$req1", "$troot", "开个话题");
    let prompts = harness.wait_prompts(1).await;
    harness.respond(&prompts[0].0, "好").await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .last()
            .is_some_and(|edit| agent_status(edit)["state"] == json!("done"))
    })
    .await;
    let (_, thread, _) = harness.room.run_statuses()[0].clone();
    assert_eq!(thread, Some("$troot".to_owned()));
    let edits = harness.room.run_edits("$status1");
    assert_eq!(
        edits.last().unwrap()["m.relates_to"],
        json!({"rel_type": "m.replace", "event_id": "$status1"}),
        "编辑关系只指向原消息，不带话题"
    );
    harness.finish().await;
}

#[tokio::test]
async fn a_restart_edits_leftover_working_status_to_uncertain() {
    let mut harness = Harness::start().await;
    let _session = start_turn(&harness, "$req1").await;
    // 一轮还挂在 Agent 手上，桥重启：遗留的 working 编辑成 failed + uncertain。
    harness.restart().await;
    wait_for(|| {
        let room = harness.room.clone();
        async move {
            room.run_edits("$status1").iter().any(|edit| {
                agent_status(edit)["state"] == json!("failed")
                    && edit["app.wonton.end"] == json!("uncertain")
            })
        }
    })
    .await;
    let edit = harness
        .room
        .run_edits("$status1")
        .into_iter()
        .find(|edit| agent_status(edit)["state"] == json!("failed"))
        .unwrap();
    assert_eq!(
        agent_status(&edit)["result"],
        json!("桥重启了，这一轮结果不确定")
    );
    harness.finish().await;
}

#[tokio::test]
async fn status_texts_come_only_from_the_category_table() {
    let harness = Harness::start().await;
    let session = start_turn(&harness, "$req1").await;
    harness
        .agent
        .raw_update(&session, tool_call("call-1", "execute"))
        .await;
    harness
        .agent
        .raw_update(&session, tool_call("call-2", "fetch"))
        .await;
    harness.respond(&session, "好了").await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .last()
            .is_some_and(|edit| agent_status(edit)["state"] == json!("done"))
    })
    .await;
    let categories: Vec<Value> = [
        "在想",
        "在写回复",
        "在读文件",
        "在改文件",
        "在删除",
        "在移动",
        "在搜索",
        "在执行命令",
        "在上网",
        "在忙别的",
        "等你确认",
        "完成，用了 2 步",
        "完成",
        "失败：Agent 拒绝了请求",
        "打断了",
        "结果不确定",
    ]
    .iter()
    .map(|text| json!(text))
    .collect();
    let mut messages: Vec<Value> = harness
        .room
        .run_statuses()
        .into_iter()
        .map(|(_, _, content)| content)
        .collect();
    messages.extend(harness.room.run_edits("$status1"));
    for content in &messages {
        assert!(
            categories.contains(&content["body"]),
            "body 只来自类别文字表：{}",
            content["body"]
        );
        if let Some(step) = agent_status(content).get("step") {
            assert!(categories.contains(step), "step 只来自类别文字表：{step}");
        }
        let wire = content.to_string();
        assert!(!wire.contains("/tmp"), "路径不进状态消息：{wire}");
        assert!(!wire.contains("rm -rf"), "命令不进状态消息：{wire}");
        assert!(!wire.contains("title"), "标题不进状态消息：{wire}");
    }
    harness.finish().await;
}

#[tokio::test]
async fn a_plan_update_sets_progress() {
    let harness = Harness::start().await;
    let session = start_turn(&harness, "$req1").await;
    harness
        .agent
        .raw_update(
            &session,
            json!({
                "sessionUpdate": "plan",
                "entries": [
                    {"content": "第一步", "status": "completed"},
                    {"content": "第二步", "status": "pending"}
                ]
            }),
        )
        .await;
    // 进度随下一次状态值编辑一起出去（等回答的 waiting 编辑不受 3 秒合并限制）。
    let _id = harness.agent.ask_permission(&session, "src/a.rs").await;
    wait_until(|| harness.cards.pending() == 1).await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .iter()
            .any(|edit| agent_status(edit)["state"] == json!("waiting"))
    })
    .await;
    let waiting = harness
        .room
        .run_edits("$status1")
        .into_iter()
        .find(|edit| agent_status(edit)["state"] == json!("waiting"))
        .unwrap();
    assert_eq!(agent_status(&waiting)["progress"], json!(0.5));
    harness.finish().await;
}

#[tokio::test]
async fn the_wire_format_matches_the_shared_fixtures() {
    let harness = Harness::start().await;
    // 话题里的一轮：发（带话题回退）→ 两次读文件 → 权限等待 → 回答后回到干活 → 完成。
    harness.send_in_thread("$req1", "$troot", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let session = prompts[0].0.clone();
    harness
        .agent
        .raw_update(&session, tool_call("call-1", "read"))
        .await;
    harness
        .agent
        .raw_update(&session, tool_call("call-2", "read"))
        .await;
    let _id = harness.agent.ask_permission(&session, "src/a.rs").await;
    wait_until(|| harness.cards.pending() == 1).await;
    harness.cards.answer("allow_once");
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .iter()
            .any(|edit| agent_status(edit)["state"] == json!("working"))
    })
    .await;
    harness.respond(&session, "答案").await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status1")
            .last()
            .is_some_and(|edit| agent_status(edit)["state"] == json!("done"))
    })
    .await;

    let (_, thread, send) = harness.room.run_statuses()[0].clone();
    assert_eq!(thread, Some("$troot".to_owned()));
    assert_eq!(
        send,
        serde_json::from_str::<Value>(include_str!("../../../fixtures/run-status/send.json"))
            .unwrap()
    );
    let edits = harness.room.run_edits("$status1");
    // 编辑序：等回答（waiting）→ 回答后回到干活（working）→ 完成（done）。
    let resumed = edits
        .iter()
        .find(|edit| agent_status(edit)["state"] == json!("working"))
        .expect("回答后回到干活的编辑");
    assert_eq!(
        resumed,
        &serde_json::from_str::<Value>(include_str!("../../../fixtures/run-status/edit.json"))
            .unwrap()
    );
    assert_eq!(
        edits.last().unwrap(),
        &serde_json::from_str::<Value>(include_str!("../../../fixtures/run-status/done.json"))
            .unwrap()
    );

    // 主对话里的一轮：执行一次工具后被主人打断。
    let session = start_turn(&harness, "$req2").await;
    harness
        .agent
        .raw_update(&session, tool_call("call-3", "execute"))
        .await;
    harness.agent.respond_prompt_as(0, "", "cancelled").await;
    wait_until(|| {
        harness
            .room
            .run_edits("$status2")
            .last()
            .is_some_and(|edit| agent_status(edit)["state"] == json!("failed"))
    })
    .await;
    assert_eq!(
        *harness.room.run_edits("$status2").last().unwrap(),
        serde_json::from_str::<Value>(include_str!("../../../fixtures/run-status/failed.json"))
            .unwrap()
    );
    harness.finish().await;
}
