//! 主人的确认：只读直接执行，改数据要先确认，回答要对上卡片上的按钮。

use super::*;

#[tokio::test]
async fn read_only_tools_run_without_asking_the_owner() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let result = s
        .call("app-example-notes__search", json!({"name": "a"}))
        .await;
    assert!(!failed(&result), "{result:?}");
    assert_eq!(text(&result), "找到 a.txt");
    assert!(
        s.cards.posted.lock().unwrap().is_empty(),
        "只读工具不发卡片"
    );
    let unknown = s.call("app-example-notes__nope", json!({})).await;
    assert!(failed(&unknown));
    let decisions: Vec<Value> = s.audit().iter().map(|l| l["decision"].clone()).collect();
    assert_eq!(decisions, [json!("allowed"), json!("denied")]);
}

/// 同为同意的按钮靠 `choice_id` 区分：回答要对得上卡片上的某一个（ADR-0035）。
#[tokio::test]
async fn an_answer_has_to_match_a_button_on_the_card() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let button = |answer, id: &str| CardChoice {
        answer,
        label: id.to_owned(),
        choice_id: Some(id.to_owned()),
    };
    let draft = Draft {
        title: "Agent 想运行 ls".to_owned(),
        lines: Vec::new(),
        choices: vec![
            button(Answer::Approve, "once"),
            button(Answer::Approve, "always"),
            button(Answer::Reject, "no"),
        ],
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
        permissions: Vec::new(),
        devices: Vec::new(),
    };
    let (bus, caller) = (s.bus.clone(), s.caller.id.clone());
    let asked = tokio::spawn(async move { bus.confirmations().ask(&caller, draft).await.is_ok() });
    let card = s.cards.wait_for(1).await;
    let answer = |answer, choice| s.bus.confirmations().answer(&card.id, answer, choice);

    for (wrong_answer, wrong_choice) in [
        (Answer::Approve, None),
        (Answer::Approve, Some("nope")),
        (Answer::Approve, Some("no")),
        (Answer::Reject, Some("once")),
    ] {
        assert_eq!(
            answer(wrong_answer, wrong_choice),
            Err(AnswerError::UnknownAnswer),
            "{wrong_answer:?} {wrong_choice:?}"
        );
    }
    assert_eq!(
        answer(Answer::Approve, Some("always")),
        Ok(ConfirmationState::Running)
    );
    assert!(asked.await.unwrap());
}

#[tokio::test]
async fn writes_run_only_after_the_owner_approves() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let call = s.spawn("app-example-notes__delete", json!({"name": "b"}));
    let card = s.cards.wait_for(1).await;
    assert_eq!(card.state, ConfirmationState::Pending);
    assert!(
        s.notes.deleted.lock().unwrap().is_empty(),
        "主人回答之前不执行"
    );
    assert_eq!(
        s.answer(&card.id, Answer::ApproveWider),
        Err(AnswerError::UnknownAnswer),
        "卡片上没有的按钮不算回答"
    );
    assert_eq!(
        s.answer(&card.id, Answer::Approve),
        Ok(ConfirmationState::Running)
    );
    let result = call.await.unwrap();
    assert_eq!(text(&result), "删掉了 b.txt");
    assert_eq!(*s.notes.deleted.lock().unwrap(), ["b"]);
    assert_eq!(
        s.bus.confirmations().card(&card.id).map(|card| card.state),
        Some(ConfirmationState::Succeeded {
            message: crate::l10n::text(&s.bus.owner_languages(), "chat-tool-ran", &[]),
        }),
        "执行完的卡片是结果"
    );
    assert_eq!(
        s.answer(&card.id, Answer::Reject),
        Err(AnswerError::AlreadyAnswered)
    );
    assert_eq!(
        s.answer("nope", Answer::Approve),
        Err(AnswerError::NotFound)
    );
}

#[tokio::test]
async fn rejected_and_expired_requests_never_run() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_millis(400)).await;
    let call = s.spawn("app-example-notes__delete", json!({"name": "c"}));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Reject).unwrap();
    let result = call.await.unwrap();
    assert!(
        failed(&result) && text(&result).contains("拒绝"),
        "{result:?}"
    );
    assert_eq!(
        s.bus.confirmations().card(&card.id).map(|card| card.state),
        Some(ConfirmationState::Rejected)
    );

    let result = s
        .call("app-example-notes__delete", json!({"name": "d"}))
        .await;
    assert!(
        failed(&result) && text(&result).contains("过期"),
        "{result:?}"
    );
    let expired = s.cards.wait_for(2).await;
    assert_eq!(
        s.bus
            .confirmations()
            .card(&expired.id)
            .map(|card| card.state),
        Some(ConfirmationState::Expired)
    );
    assert_eq!(
        s.answer(&expired.id, Answer::Approve),
        Err(AnswerError::Expired)
    );
    assert!(s.notes.deleted.lock().unwrap().is_empty());
    let decisions: Vec<Value> = s.audit().iter().map(|l| l["decision"].clone()).collect();
    assert_eq!(decisions, [json!("rejected"), json!("expired")]);
}

#[tokio::test]
async fn without_a_card_nothing_runs() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    s.cards.broken.store(true, Ordering::SeqCst);
    let result = s
        .call("app-example-notes__delete", json!({"name": "e"}))
        .await;
    assert!(
        failed(&result) && text(&result).contains("没能请你确认"),
        "{result:?}"
    );
    s.cards.broken.store(false, Ordering::SeqCst);
    // 同一个插件最多三个请求同时等主人。
    let waiting: Vec<_> = (0..3)
        .map(|i| s.spawn("app-example-notes__delete", json!({"name": i})))
        .collect();
    s.cards.wait_for(3).await;
    let result = s
        .call("app-example-notes__delete", json!({"name": "f"}))
        .await;
    assert!(
        failed(&result) && text(&result).contains("等你确认"),
        "{result:?}"
    );
    for card in s.cards.posted.lock().unwrap().iter() {
        s.answer(&card.id, Answer::Reject).unwrap();
    }
    for call in waiting {
        assert!(failed(&call.await.unwrap()));
    }
    assert!(s.notes.deleted.lock().unwrap().is_empty());
}
