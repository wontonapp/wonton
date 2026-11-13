use rmcp::model::NumberOrString;
use serde_json::{Value, json};

use super::handoff_test_support::{A, B, Bed, C, bed, bot, success};

fn args(target: &str, message: &str) -> Value {
    json!({"target_bot":bot(target),"message":message})
}

fn request(bed: &Bed, id: u32) -> super::Caller {
    let mut caller = bed.caller.clone();
    caller.request_id = Some(NumberOrString::Number(id.into()));
    caller
}

fn failure(result: &rmcp::model::CallToolResult) -> Option<&str> {
    result.meta.as_ref()?.0.get("app.wonton/failure")?.as_str()
}

fn session(opened: &Value) -> &str {
    opened["session"].as_str().unwrap()
}

#[tokio::test]
async fn identical_live_message_with_a_new_request_id_reuses_the_session() {
    let bed = bed(true, "@owner:test").await;
    let first = bed.open("修登录页").await;
    let repeated = success(
        &bed.call(&request(&bed, 2), "wonton__hand_off", args(B, "修登录页"))
            .await,
    );
    assert_eq!(repeated, first);
    bed.provider.recording.wait_prompt(0).await;
    assert_eq!(bed.provider.recording.method_count("session/new"), 1);
    assert_eq!(bed.provider.recording.method_count("session/prompt"), 1);
    assert_eq!(
        bed.chains
            .ledger()
            .row(bed.caller.chain.as_ref().unwrap())
            .unwrap()
            .rounds,
        2
    );
    bed.finish(session(&first), 0).await;
}

#[tokio::test]
async fn identical_message_after_completion_opens_a_new_sibling_but_old_id_still_replays() {
    let bed = bed(true, "@owner:test").await;
    let first = bed.open("修登录页").await;
    bed.finish(session(&first), 0).await;
    let replayed = success(
        &bed.call(&bed.caller, "wonton__hand_off", args(B, "修登录页"))
            .await,
    );
    assert_eq!(replayed, first);
    let second = success(
        &bed.call(&request(&bed, 2), "wonton__hand_off", args(B, "修登录页"))
            .await,
    );
    assert_ne!(second["session"], first["session"]);
    assert_eq!(second["chain"], first["chain"]);
    let info = bed.chains.ledger().session_chain(session(&second)).unwrap();
    assert_eq!(info.layer, 2);
    assert_eq!(info.parents, ["caller-session"]);
    assert_eq!(bed.provider.recording.method_count("session/new"), 2);
    assert_eq!(bed.cards.posted.lock().unwrap().len(), 1);
    bed.finish(session(&second), 1).await;
}

#[tokio::test]
async fn changing_one_character_opens_another_live_sibling() {
    let bed = bed(true, "@owner:test").await;
    let first = bed.open("修登录页").await;
    let second = success(
        &bed.call(&request(&bed, 2), "wonton__hand_off", args(B, "修登录栏"))
            .await,
    );
    assert_ne!(second["session"], first["session"]);
    assert_eq!(second["chain"], first["chain"]);
    bed.provider.recording.wait_prompt(1).await;
    for opened in [&first, &second] {
        let info = bed.chains.ledger().session_chain(session(opened)).unwrap();
        assert!(info.live);
        assert_eq!(info.layer, 2);
        assert_eq!(info.parents, ["caller-session"]);
    }
    assert_eq!(bed.provider.recording.method_count("session/new"), 2);
    assert_eq!(bed.provider.recording.method_count("session/prompt"), 2);
    assert_eq!(
        bed.chains
            .ledger()
            .row(bed.caller.chain.as_ref().unwrap())
            .unwrap()
            .rounds,
        3
    );
    bed.finish(session(&first), 0).await;
    bed.finish(session(&second), 1).await;
}

#[tokio::test]
async fn sibling_bot_is_not_an_ancestor_and_its_child_uses_the_exact_parent_layer() {
    let bed = bed(true, "@owner:test").await;
    let first_b = bed.open("修登录页").await;
    bed.route(A, C);
    let sibling_c = bed.approved(&request(&bed, 2), C, "检查登录页", 2).await;
    bed.route(C, B);
    let c = bed.child_caller(C, session(&sibling_c), 3);
    let deeper_b = bed.approved(&c, B, "检查表单", 3).await;
    let info = bed
        .chains
        .ledger()
        .session_chain(session(&deeper_b))
        .unwrap();
    assert_eq!(info.chain, first_b["chain"].as_str().unwrap());
    assert_eq!(info.parents, [session(&sibling_c)]);
    assert_eq!(info.layer, 3);
    bed.route(B, C);
    let shallow_b = bed.child_caller(B, session(&first_b), 4);
    let deeper_c = bed.approved(&shallow_b, C, "检查按钮", 4).await;
    let info = bed
        .chains
        .ledger()
        .session_chain(session(&deeper_c))
        .unwrap();
    assert_eq!(info.parents, [session(&first_b)]);
    assert_eq!(info.layer, 3);
    assert_eq!(bed.provider.recording.method_count("session/new"), 2);
    assert_eq!(bed.worker.recording.method_count("session/new"), 2);
    bed.finish(session(&first_b), 0).await;
    bed.finish(session(&sibling_c), 0).await;
    bed.finish(session(&deeper_b), 1).await;
    bed.finish(session(&deeper_c), 1).await;
}

#[tokio::test]
async fn self_and_all_ancestors_are_rejected_before_any_connection_or_confirmation() {
    let bed = bed(true, "@owner:test").await;
    let first_b = bed.open("修登录页").await;
    bed.route(B, C);
    let b = bed.child_caller(B, session(&first_b), 2);
    let child_c = bed.approved(&b, C, "检查表单", 2).await;
    let c = bed.child_caller(C, session(&child_c), 3);
    for (id, (caller, target)) in [
        (&bed.caller, A),
        (&b, B),
        (&b, A),
        (&c, C),
        (&c, A),
        (&c, B),
    ]
    .into_iter()
    .enumerate()
    {
        let mut caller = caller.clone();
        caller.request_id = Some(NumberOrString::Number(100 + id as i64));
        let result = bed
            .call(&caller, "wonton__hand_off", args(target, "不要绕圈"))
            .await;
        assert_eq!(failure(&result), Some("invalid_input"), "{result:?}");
    }
    assert_eq!(
        bed.provider.recording.connections() + bed.worker.recording.connections(),
        2
    );
    assert_eq!(bed.cards.posted.lock().unwrap().len(), 2);
    assert_eq!(
        bed.chains
            .ledger()
            .row(bed.caller.chain.as_ref().unwrap())
            .unwrap()
            .rounds,
        3
    );
    bed.finish(session(&first_b), 0).await;
    bed.finish(session(&child_c), 0).await;
}

#[tokio::test]
async fn fourth_layer_is_rejected_without_blocking_a_new_second_layer_task() {
    let bed = bed(true, "@owner:test").await;
    let first_b = bed.open("修登录页").await;
    bed.route(B, C);
    let b = bed.child_caller(B, session(&first_b), 2);
    let child_c = bed.approved(&b, C, "检查表单", 2).await;
    let c = bed.child_caller(C, session(&child_c), 3);
    let result = bed
        .call(&c, "wonton__hand_off", args("com.example.fourth", "第四层"))
        .await;
    assert_eq!(failure(&result), Some("limit_reached"), "{result:?}");
    assert_eq!(
        bed.provider.recording.connections() + bed.worker.recording.connections(),
        2
    );
    let second_b = success(
        &bed.call(&request(&bed, 4), "wonton__hand_off", args(B, "另一件事"))
            .await,
    );
    assert_eq!(
        bed.chains
            .ledger()
            .session_chain(session(&second_b))
            .unwrap()
            .layer,
        2
    );
    bed.finish(session(&first_b), 0).await;
    bed.finish(session(&child_c), 0).await;
    bed.finish(session(&second_b), 1).await;
}
