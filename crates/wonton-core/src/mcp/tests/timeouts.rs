//! 调用的期限和取消：确认过的调用期满结果不确定，Agent 取消要传到插件。

use super::*;

/// 确认过的调用到期了：副作用可能已经发生，卡片和给 Agent 的结果都说不确定，不说失败。Agent 断开时它不跟着取消（ADR-0016）。
#[tokio::test]
async fn a_confirmed_call_that_times_out_leaves_the_card_uncertain() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let hanging = Notes::default();
    hanging.hang.store(true, Ordering::SeqCst);
    let manifest = NOTES
        .replace("name: notes", "name: hanging")
        .replace("app.example.notes", "app.example.hanging")
        .replace("streamable-http}", "streamable-http, tool_idle_seconds: 1}");
    let manifest = PluginPackage::parse(&manifest, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    let id = manifest.id.clone();
    s.manager
        .register_for_test(manifest, PluginStatus::Running, None);
    s.manager
        .mcp_for_test(&id, serve_notes(hanging.clone()).await);

    let call = s.spawn("app-example-hanging__delete", json!({"name": "a"}));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    // 不写外层期限的话，空闲期限被忽略时这里要等 30 分钟的总期限。
    let result = tokio::time::timeout(Duration::from_secs(30), call)
        .await
        .expect("空闲期限到了就要有结果")
        .unwrap();
    assert!(failed(&result), "{result:?}");
    assert!(text(&result).contains("是否已执行不确定"), "{result:?}");
    let meta = result.meta.as_ref().unwrap();
    assert_eq!(meta.0["app.wonton/failure"], "tool_timed_out");
    assert!(
        matches!(
            s.bus.confirmations().card(&card.id).map(|card| card.state),
            Some(ConfirmationState::Uncertain { .. })
        ),
        "执行完的卡片标结果不确定"
    );
    assert_eq!(s.audit().last().unwrap()["decision"], "approved");
}

/// Agent 取消了只读的调用（或者断开了）：取消传给插件，不留一个没人要结果的调用在那里跑。
#[tokio::test]
async fn an_agent_cancelling_a_read_only_call_cancels_it_in_the_plugin() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    s.notes.hang.store(true, Ordering::SeqCst);
    let request = CallToolRequestParams::new("app-example-notes__search".to_owned());
    let cancelled = tokio::time::sleep(Duration::from_millis(100));
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        s.bus.call(&s.caller, request, cancelled),
    )
    .await
    .expect("取消之后不再等插件");
    assert!(failed(&result), "{result:?}");
    for _ in 0..500 {
        if s.notes.cancelled.load(Ordering::SeqCst) == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("插件没有收到取消");
}
