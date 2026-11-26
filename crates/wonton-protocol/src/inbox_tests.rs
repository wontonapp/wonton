//! 收件箱协议的契约测试：线上形状、未知值、旧数据缺字段、字段上限。

use serde_json::{Value, json};

use crate::inbox::*;
use crate::inbox_validate::*;

fn option(id: &str, label: &str) -> InboxOption {
    InboxOption {
        id: id.into(),
        label: label.into(),
        description: None,
        recommended: false,
        preview: None,
    }
}

fn question(id: &str) -> InboxQuestion {
    InboxQuestion {
        id: id.into(),
        header: "目标盘".into(),
        question: "照片备份到哪块硬盘？".into(),
        multi_select: false,
        options: vec![option("nas", "NAS"), option("local", "主机自带的硬盘")],
        note_enabled: true,
        skippable: false,
    }
}

fn item(kind: InboxType, payload: InboxPayload) -> InboxItem {
    InboxItem {
        id: format!("{}-01J", kind.prefix().unwrap_or('x')),
        kind,
        status: InboxStatus::Pending,
        created_at_ms: 1,
        expires_at_ms: Some(2),
        sender: InboxSender {
            kind: InboxSenderKind::Agent,
            id: "app.hermes".into(),
            display_name: "Hermes".into(),
            avatar: None,
        },
        recipient: InboxRecipient::Person {
            username: "owner".into(),
        },
        title: "照片备份".into(),
        body: String::new(),
        source: None,
        level: None,
        blocking: true,
        dedupe_key: None,
        progress: None,
        snooze_until_ms: None,
        read_by: Vec::new(),
        answered_by: None,
        answered_at_ms: None,
        outcome: None,
        result: None,
        payload,
        raw_payload: None,
        answer: None,
        superseded_answers: Vec::new(),
    }
}

fn choice_item() -> InboxItem {
    item(
        InboxType::Choice,
        InboxPayload::Choice(ChoicePayload {
            questions: vec![question("q0")],
        }),
    )
}

/// 契约：ADI-0119 第 3 节的问题卡 JSON 原样能解析，答案按 question_id。
#[test]
fn the_documented_ask_shape_round_trips() {
    let ask = json!({
        "title": "照片备份",
        "questions": [{
            "id": "q0",
            "header": "目标盘",
            "question": "照片备份到哪块硬盘？",
            "multi_select": false,
            "options": [
                {"id": "nas", "label": "NAS", "description": "容量大，慢一些", "recommended": true,
                 "preview": "<div>…</div>"},
                {"id": "local", "label": "主机自带的硬盘"}
            ],
            "note_enabled": true,
            "skippable": true
        }],
        "blocking": true,
        "expires_in_ms": 600000,
        "dedupe_key": "photo-backup-1"
    });
    let parsed: InboxAskRequest = serde_json::from_value(ask).unwrap();
    assert_eq!(parsed.questions.len(), 1);
    assert_eq!(parsed.dedupe_key.as_deref(), Some("photo-backup-1"));
    assert!(parsed.blocking);
    validate_choice(&ChoicePayload {
        questions: parsed.questions,
    })
    .unwrap();

    let answer = json!({
        "answer_id": "01J-answer",
        "action": "submit",
        "answers": [{"question_id": "q0", "selected": ["nas"], "note": "用 2 号盘"}]
    });
    let answer: InboxAnswerInput = serde_json::from_value(answer).unwrap();
    assert_eq!(answer.answers[0].selected, ["nas"]);
    validate_answer(&choice_item(), &answer).unwrap();
}

/// 契约：未知类型和未知状态落到 `Other`，条目照常解析出已知字段。
#[test]
fn unknown_types_and_statuses_fall_back_to_other() {
    assert_eq!(
        serde_json::from_value::<InboxType>(json!("form")).unwrap(),
        InboxType::Other
    );
    assert_eq!(
        serde_json::from_value::<InboxStatus>(json!("snoozed")).unwrap(),
        InboxStatus::Other
    );
    assert_eq!(
        serde_json::from_value::<InboxAnswerAction>(json!("reject")).unwrap(),
        InboxAnswerAction::Other
    );
    assert_eq!(InboxType::Other.prefix(), None);
    assert_eq!(InboxType::from_prefix('q'), Some(InboxType::Choice));
    assert_eq!(InboxType::from_prefix('x'), None);
}

/// 契约：旧核心的条目缺可选字段也能解析，新 App 把缺的当默认值。
#[test]
fn old_items_without_optional_fields_parse() {
    let bare = json!({
        "id": "q-1",
        "type": "choice",
        "status": "pending",
        "created_at_ms": 1,
        "sender": {"kind": "agent", "id": "app.hermes", "display_name": "Hermes"},
        "title": "照片备份",
        "payload": {"type": "choice", "payload": {"questions": [{
            "id": "q0", "header": "目标盘", "question": "备份到哪？",
            "options": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]
        }]}}
    });
    let parsed: InboxItem = serde_json::from_value(bare).unwrap();
    assert_eq!(parsed.recipient, InboxRecipient::Other);
    assert_eq!(parsed.expires_at_ms, None);
    assert!(!parsed.blocking);
    assert!(parsed.answer.is_none());
    assert_eq!(parsed.payload.kind(), InboxType::Choice);
}

/// 契约：题目和答案的上限按字符数算，超了报字段，不截断。
#[test]
fn limits_name_the_field() {
    let mut long = choice_item();
    let InboxPayload::Choice(payload) = &mut long.payload else {
        unreachable!()
    };
    payload.questions[0].question = "问".repeat(MAX_INBOX_QUESTION_CHARS + 1);
    let err = validate_item(&long).unwrap_err();
    assert_eq!(err.field(), "questions.question");

    let mut too_many = choice_item();
    let InboxPayload::Choice(payload) = &mut too_many.payload else {
        unreachable!()
    };
    payload.questions = (0..=MAX_INBOX_QUESTIONS)
        .map(|i| question(&format!("q{i}")))
        .collect();
    assert_eq!(
        validate_item(&too_many).unwrap_err().field(),
        "payload.questions"
    );

    let mut one_option = choice_item();
    let InboxPayload::Choice(payload) = &mut one_option.payload else {
        unreachable!()
    };
    payload.questions[0].options = vec![option("only", "唯一选项")];
    assert_eq!(
        validate_item(&one_option).unwrap_err().field(),
        "questions.options"
    );
}

/// 契约：页面必须二选一来源，网络名单只收 https/wss 主机。
#[test]
fn pages_need_one_source_and_clean_domains() {
    let page = |domains: Vec<String>| {
        item(
            InboxType::Page,
            InboxPayload::Page(PagePayload {
                resource_uri: Some("ui://photos/report/42".into()),
                file_id: None,
                caption: "这周的备份报告".into(),
                network_domains: domains,
                actions: Vec::new(),
            }),
        )
    };
    validate_item(&page(vec![])).unwrap();
    validate_item(&page(vec!["https://cdn.example.com".into()])).unwrap();
    validate_item(&page(vec!["wss://*.example.com:8443".into()])).unwrap();
    for bad in [
        "http://example.com",
        "https://example.com/path",
        "https://*example.com",
        "ftp://example.com",
        "https://",
    ] {
        assert!(
            validate_network_domains(&[bad.into()]).is_err(),
            "{bad} 应该被拒"
        );
    }
    let both = item(
        InboxType::Page,
        InboxPayload::Page(PagePayload {
            resource_uri: Some("ui://x".into()),
            file_id: Some("abc".into()),
            caption: "x".into(),
            network_domains: Vec::new(),
            actions: Vec::new(),
        }),
    );
    assert!(validate_item(&both).is_err());
}

/// 契约：确认至少同意和拒绝两档；答案只能是给的档。
#[test]
fn confirms_carry_fixed_choices() {
    let confirm = item(
        InboxType::Confirm,
        InboxPayload::Confirm(ConfirmPayload {
            title: "安装 Immich？".into(),
            body: "需要 storage.share.photos。".into(),
            detail: None,
            risk: InboxRisk::High,
            choices: vec![
                ConfirmChoice {
                    action: ConfirmAction::Approve,
                    label: "允许".into(),
                },
                ConfirmChoice {
                    action: ConfirmAction::Deny,
                    label: "拒绝".into(),
                },
            ],
        }),
    );
    validate_item(&confirm).unwrap();
    let approve = InboxAnswerInput {
        answer_id: "a1".into(),
        action: InboxAnswerAction::Approve,
        answers: Vec::new(),
        action_id: None,
        comment: None,
    };
    validate_answer(&confirm, &approve).unwrap();
    let always = InboxAnswerInput {
        action: InboxAnswerAction::Always,
        ..approve.clone()
    };
    assert!(
        validate_answer(&confirm, &always).is_err(),
        "没给 always 档"
    );

    let mut only_approve = confirm.clone();
    let InboxPayload::Confirm(payload) = &mut only_approve.payload else {
        unreachable!()
    };
    payload.choices.truncate(1);
    assert_eq!(
        validate_item(&only_approve).unwrap_err().field(),
        "payload.choices"
    );
}

/// 契约：通知只收三种链接、知道了就行；有动作时必须给 action_id。
#[test]
fn notices_and_actions_are_strict() {
    let notice = |link: Option<&str>, action: Option<InboxAction>| {
        item(
            InboxType::Notice,
            InboxPayload::Notice(NoticePayload {
                title: "备份失败".into(),
                body: "没有空间".into(),
                level: InboxLevel::Warning,
                link: link.map(str::to_owned),
                action,
            }),
        )
    };
    validate_item(&notice(None, None)).unwrap();
    validate_item(&notice(Some("wonton://plugins/immich"), None)).unwrap();
    validate_item(&notice(Some("ftp://x"), None)).unwrap_err();
    validate_item(&notice(
        None,
        Some(InboxAction {
            id: "retry".into(),
            label: "重试".into(),
            style: InboxActionStyle::Primary,
            requires_comment: false,
        }),
    ))
    .unwrap();

    let input = InboxAnswerInput {
        answer_id: "a1".into(),
        action: InboxAnswerAction::Action,
        answers: Vec::new(),
        action_id: Some("nope".into()),
        comment: None,
    };
    assert!(validate_answer(&notice(None, None), &input).is_err());
}

/// 契约：答案里的题目和选项必须对得上条目；单选题最多一项。
#[test]
fn answers_must_match_the_questions() {
    let input = |answers: Vec<InboxQuestionAnswer>| InboxAnswerInput {
        answer_id: "a1".into(),
        action: InboxAnswerAction::Submit,
        answers,
        action_id: None,
        comment: None,
    };
    let answer = |question_id: &str, selected: &[&str]| InboxQuestionAnswer {
        question_id: question_id.into(),
        selected: selected.iter().map(|s| (*s).to_owned()).collect(),
        ..Default::default()
    };
    validate_answer(&choice_item(), &input(vec![answer("q0", &["nas"])])).unwrap();
    assert!(validate_answer(&choice_item(), &input(vec![answer("q1", &["nas"])])).is_err());
    assert!(validate_answer(&choice_item(), &input(vec![answer("q0", &["nope"])])).is_err());
    assert!(validate_answer(&choice_item(), &input(vec![answer("q0", &[])]),).is_err());
    assert!(
        validate_answer(
            &choice_item(),
            &input(vec![answer("q0", &["nas", "local"])])
        )
        .is_err()
    );
    validate_answer(&choice_item(), &input(Vec::new())).unwrap_err();
}

/// 契约：单选的“其他”可以直接答，不选选项也行。
#[test]
fn other_is_an_answer() {
    let input = InboxAnswerInput {
        answer_id: "a1".into(),
        action: InboxAnswerAction::Submit,
        answers: vec![InboxQuestionAnswer {
            question_id: "q0".into(),
            other: Some("移动硬盘".into()),
            ..Default::default()
        }],
        action_id: None,
        comment: None,
    };
    validate_answer(&choice_item(), &input).unwrap();
}

/// 契约：条目 id 带类型前缀，动作 id 只收小写。
#[test]
fn ids_have_a_stable_shape() {
    assert_eq!(inbox_item_path("q-1"), "/_wonton/api/inbox/q-1");
    assert_eq!(
        inbox_page_path("p-1", "tok"),
        "/_wonton/api/inbox/p-1/page?token=tok"
    );
    assert!(validate_item(&choice_item()).is_ok());
    let mut unknown = choice_item();
    unknown.id = "x-1".into();
    validate_item(&unknown).unwrap();
    let mut wrong = choice_item();
    wrong.id = "p-1".into();
    assert_eq!(
        validate_item(&wrong).unwrap_err().field(),
        "id 前缀和 type 不一致"
    );
    validate_action_id("x", "approve").unwrap();
    validate_action_id("x", "approve_2").unwrap();
    validate_action_id("x", "Approve").unwrap_err();
    validate_action_id("x", "").unwrap_err();
    validate_action_id("x", "a".repeat(33).as_str()).unwrap_err();
}

/// 契约：未知载荷类型解析成 Other；类型和载荷不一致的条目校验失败。
#[test]
fn unknown_payload_is_other() {
    let payload: InboxPayload = serde_json::from_value(json!({
        "type": "form",
        "payload": {"fields": [{"id": "name", "label": "名字"}]}
    }))
    .unwrap();
    let InboxPayload::Other(raw) = payload else {
        panic!("未知 tag 要收住载荷");
    };
    assert_eq!(raw["fields"][0]["label"], "名字");
    assert_eq!(
        serde_json::to_value(InboxPayload::Other(raw.clone())).unwrap(),
        json!({"type": "other"})
    );
    let item_json = json!({
        "id": "f-1", "type": "form", "status": "pending", "created_at_ms": 1,
        "sender": {"kind": "agent", "id": "app.hermes", "display_name": "Hermes"},
        "title": "表单",
        "payload": {"type": "form", "payload": {"fields": [{"id": "name", "label": "名字"}]}},
        "raw_payload": {"type": "form", "payload": {"fields": []}}
    });
    let item: InboxItem = serde_json::from_value(item_json).unwrap();
    assert!(matches!(item.payload, InboxPayload::Other(_)));
    assert_eq!(item.raw_payload.unwrap()["type"], "form");
    let mut mismatch = choice_item();
    mismatch.kind = InboxType::Page;
    mismatch.id = "p-1".into();
    assert_eq!(
        validate_item(&mismatch).unwrap_err().field(),
        "Page 的条目的载荷是 Choice"
    );
}

/// 契约：可空字段不出现时保持空，序列化回来也省掉。
#[test]
fn optional_fields_stay_off_the_wire() {
    let value: Value = serde_json::to_value(choice_item()).unwrap();
    assert!(value.get("level").is_none());
    assert!(value.get("answer").is_none());
    let parsed: InboxItem = serde_json::from_value(value).unwrap();
    assert_eq!(parsed, choice_item());
}

/// 契约：备注默认不收；题目没开 note_enabled 时答案带 note 要报字段。
#[test]
fn notes_are_opt_in() {
    let mut item = choice_item();
    let InboxPayload::Choice(payload) = &mut item.payload else {
        unreachable!()
    };
    payload.questions[0].note_enabled = false;
    let input = |note: &str| InboxAnswerInput {
        answer_id: "a1".into(),
        action: InboxAnswerAction::Submit,
        answers: vec![InboxQuestionAnswer {
            question_id: "q0".into(),
            selected: vec!["nas".into()],
            note: Some(note.into()),
            ..Default::default()
        }],
        action_id: None,
        comment: None,
    };
    assert_eq!(
        validate_answer(&item, &input("用 2 号盘"))
            .unwrap_err()
            .field(),
        "answers.note"
    );
    let json = json!({
        "id": "q0", "header": "目标盘", "question": "备份到哪？",
        "options": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]
    });
    let question: InboxQuestion = serde_json::from_value(json).unwrap();
    assert!(
        !question.note_enabled,
        "旧数据缺 note_enabled 时默认不收备注"
    );
    let long = "备".repeat(MAX_INBOX_NOTE_CHARS + 1);
    let InboxPayload::Choice(payload) = &mut item.payload else {
        unreachable!()
    };
    payload.questions[0].note_enabled = true;
    assert_eq!(
        validate_answer(&item, &input(&long)).unwrap_err().field(),
        "answers.note"
    );
}

/// 契约：recap 要有 1–2 个动作；有问题时必须选动作，没问题时可以知道了。
#[test]
fn recaps_need_one_or_two_actions() {
    let recap = |question: Option<&str>, actions: Vec<InboxAction>| {
        item(
            InboxType::Recap,
            InboxPayload::Recap(RecapPayload {
                summary: "已经挂载了，还缺证书。".into(),
                question: question.map(str::to_owned),
                actions,
            }),
        )
    };
    let action = |id: &str| InboxAction {
        id: id.into(),
        label: "继续".into(),
        style: InboxActionStyle::Primary,
        requires_comment: false,
    };
    assert!(validate_item(&recap(None, Vec::new())).is_err());
    validate_item(&recap(None, vec![action("a")])).unwrap();
    validate_item(&recap(None, vec![action("a"), action("b")])).unwrap();
    assert!(validate_item(&recap(None, vec![action("a"), action("b"), action("c")])).is_err());
    assert!(validate_item(&recap(Some("继续吗？"), Vec::new())).is_err());
    assert!(
        validate_item(&recap(
            Some("继续吗？"),
            vec![action("a"), action("b"), action("c")]
        ))
        .is_err()
    );
    // 动作 id/标签/样式/重复都要过 validate_actions，不能因为 recap 分支删了它就失守。
    assert!(
        validate_item(&recap(
            None,
            vec![InboxAction {
                id: "BAD".into(),
                label: "坏".into(),
                style: InboxActionStyle::Default,
                requires_comment: false
            }]
        ))
        .is_err()
    );
    assert!(
        validate_item(&recap(
            None,
            vec![InboxAction {
                id: "a".into(),
                label: String::new(),
                style: InboxActionStyle::Default,
                requires_comment: false
            }]
        ))
        .is_err()
    );
    assert!(
        validate_item(&recap(
            None,
            vec![InboxAction {
                id: "a".into(),
                label: "A".into(),
                style: InboxActionStyle::Other,
                requires_comment: false
            }]
        ))
        .is_err()
    );
    assert!(validate_item(&recap(None, vec![action("dup"), action("dup")])).is_err());

    let ack = InboxAnswerInput {
        answer_id: "a1".into(),
        action: InboxAnswerAction::Acknowledge,
        answers: Vec::new(),
        action_id: None,
        comment: None,
    };
    assert!(validate_answer(&recap(Some("继续吗？"), vec![action("go")]), &ack).is_err());
    validate_answer(&recap(None, vec![action("go")]), &ack).unwrap();
}

/// 契约：page 有动作时不能只知道了；没动作时知道了就行。
#[test]
fn pages_with_actions_need_an_action() {
    let page = |actions: Vec<InboxAction>| {
        item(
            InboxType::Page,
            InboxPayload::Page(PagePayload {
                resource_uri: Some("ui://x".into()),
                file_id: None,
                caption: "报告".into(),
                network_domains: Vec::new(),
                actions,
            }),
        )
    };
    let ack = InboxAnswerInput {
        answer_id: "a1".into(),
        action: InboxAnswerAction::Acknowledge,
        answers: Vec::new(),
        action_id: None,
        comment: None,
    };
    validate_answer(&page(Vec::new()), &ack).unwrap();
    let with_action = page(vec![InboxAction {
        id: "approve".into(),
        label: "通过".into(),
        style: InboxActionStyle::Primary,
        requires_comment: false,
    }]);
    assert!(validate_answer(&with_action, &ack).is_err());
}

/// 契约：确认请求默认阻塞，可以显式关掉。
#[test]
fn confirm_requests_block_by_default() {
    let request: InboxConfirmRequest = serde_json::from_value(json!({
        "title": "装 Immich？", "body": "要 storage.share.photos"
    }))
    .unwrap();
    assert!(request.blocking);
}
