//! 收件箱行为测试：给定操作序列，断言 ViewModel 和对外副作用（ADR-0119 第 3 节）。

use std::sync::Arc;

use wonton_protocol::{
    ChoicePayload, InboxAnswerAction, InboxAnswerInput, InboxOption, InboxQuestion,
    InboxQuestionAnswer, InboxRecipient, InboxSender, InboxSenderKind, InboxSnoozePreset,
    InboxStatus, InboxType, PagePayload,
};

use super::page::FixedClock;
use super::store::{CreateInput, Inbox, InboxError};

fn inbox() -> (Arc<Inbox>, FixedClock) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_owned();
    std::mem::forget(dir);
    let clock = FixedClock::new(1_000_000);
    let inbox = Inbox::for_test(&path).set_clock(Arc::new(clock.clone()));
    (inbox, clock)
}

fn question() -> InboxQuestion {
    InboxQuestion {
        id: "q0".into(),
        header: "目标盘".into(),
        question: "备份到哪？".into(),
        multi_select: false,
        options: vec![
            InboxOption {
                id: "nas".into(),
                label: "NAS".into(),
                description: None,
                recommended: false,
                preview: None,
            },
            InboxOption {
                id: "local".into(),
                label: "本地".into(),
                description: None,
                recommended: false,
                preview: None,
            },
        ],
        note_enabled: true,
        skippable: false,
    }
}

fn create(inbox: &Inbox) -> super::store::CreateReply {
    inbox
        .create(CreateInput {
            kind: InboxType::Choice,
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
            level: None,
            source: None,
            blocking: true,
            dedupe_key: None,
            payload: wonton_protocol::InboxPayload::Choice(ChoicePayload {
                questions: vec![question()],
            }),
            expires_in_ms: None,
            page: None,
        })
        .unwrap()
}

fn answer_input(selected: &str) -> InboxAnswerInput {
    InboxAnswerInput {
        answer_id: format!("a-{selected}"),
        action: InboxAnswerAction::Submit,
        answers: vec![InboxQuestionAnswer {
            question_id: "q0".into(),
            selected: vec![selected.into()],
            ..Default::default()
        }],
        action_id: None,
        comment: None,
    }
}

/// 发卡 → 列表出现 → 作答 → 阻塞的发送方收到结构化答案。
#[tokio::test]
async fn a_card_appears_then_the_sender_gets_the_answer() {
    let (inbox, clock) = inbox();
    let created = create(&inbox);
    let list = inbox.list("owner", clock.get());
    assert_eq!(list.items.len(), 1);
    assert!(list.items[0].needs_answer);
    assert_eq!(list.counts.needing, 1);

    let outcome = inbox
        .answer("owner", &created.item.id, answer_input("nas"))
        .unwrap();
    assert_eq!(outcome.item.status, InboxStatus::Handled);
    assert_eq!(outcome.item.answered_by.as_deref(), Some("owner"));
    let answer = created.waiter.unwrap().await.unwrap();
    assert_eq!(answer.answers[0].selected, ["nas"]);
    assert!(inbox.list("owner", clock.get()).items[0].status == InboxStatus::Handled);
}

/// 异步条目第一次答完 60 秒内可以换答案，超了锁死。
#[tokio::test]
async fn an_async_answer_can_be_changed_within_sixty_seconds() {
    let (inbox, clock) = inbox();
    let mut created = create(&inbox);
    created.item.blocking = false;
    // 先把它改成异步：重新建一条
    let async_item = inbox
        .create(CreateInput {
            kind: InboxType::Choice,
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
            level: None,
            source: None,
            blocking: false,
            dedupe_key: None,
            payload: wonton_protocol::InboxPayload::Choice(ChoicePayload {
                questions: vec![question()],
            }),
            expires_in_ms: None,
            page: None,
        })
        .unwrap();
    let id = async_item.item.id.clone();
    let _ = id;
    inbox
        .answer("owner", &async_item.item.id, answer_input("nas"))
        .unwrap();
    clock.set(clock.get() + 10_000);
    let mut second = answer_input("local");
    second.answer_id = "a2".into();
    let outcome = inbox.answer("owner", &async_item.item.id, second).unwrap();
    assert!(outcome.superseded);
    assert_eq!(outcome.item.superseded_answers.len(), 1);
    clock.set(clock.get() + 61_000);
    let mut third = answer_input("nas");
    third.answer_id = "a3".into();
    assert_eq!(
        inbox
            .answer("owner", &async_item.item.id, third)
            .unwrap_err(),
        InboxError::AnswerLocked
    );
}

/// 过期后不能再答，sweep 把它标成 expired。
#[tokio::test]
async fn expiry_is_a_terminal_state() {
    let (inbox, clock) = inbox();
    let created = inbox
        .create(CreateInput {
            kind: InboxType::Choice,
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
            level: None,
            source: None,
            blocking: false,
            dedupe_key: None,
            payload: wonton_protocol::InboxPayload::Choice(ChoicePayload {
                questions: vec![question()],
            }),
            expires_in_ms: Some(1000),
            page: None,
        })
        .unwrap();
    clock.set(clock.get() + 2000);
    inbox.sweep(clock.get());
    assert_eq!(
        inbox
            .view("owner", &created.item.id, clock.get())
            .unwrap()
            .item
            .status,
        InboxStatus::Expired
    );
    assert_eq!(
        inbox
            .answer("owner", &created.item.id, answer_input("nas"))
            .unwrap_err(),
        InboxError::Expired
    );
}

/// 发送方撤回后，阻塞的发送方拿到一条“已撤回”。
#[tokio::test]
async fn cancel_wakes_the_blocked_sender() {
    let (inbox, _clock) = inbox();
    let created = create(&inbox);
    inbox
        .cancel_by_sender("app.hermes", &created.item.id, Some("任务结束".into()))
        .unwrap();
    let answer = created.waiter.unwrap().await.unwrap();
    assert_eq!(answer.answer_id, "withdrawn");
}

/// 挂着的 choice/confirm/recap 有上限，page 另算。
#[tokio::test]
async fn pending_limits_are_enforced_per_sender() {
    let (inbox, _clock) = inbox();
    for _ in 0..5 {
        create(&inbox);
    }
    assert_eq!(
        inbox.create(create_input()).unwrap_err(),
        InboxError::TooManyPending
    );
    for _ in 0..3 {
        let mut input = create_input();
        input.kind = InboxType::Page;
        input.blocking = false;
        input.payload = wonton_protocol::InboxPayload::Page(PagePayload {
            resource_uri: Some("ui://x".into()),
            file_id: None,
            caption: "报告".into(),
            network_domains: Vec::new(),
            actions: Vec::new(),
        });
        inbox.create(input).unwrap();
    }
    let mut page = create_input();
    page.kind = InboxType::Page;
    page.blocking = false;
    page.payload = wonton_protocol::InboxPayload::Page(PagePayload {
        resource_uri: Some("ui://x".into()),
        file_id: None,
        caption: "报告".into(),
        network_domains: Vec::new(),
        actions: Vec::new(),
    });
    assert_eq!(inbox.create(page).unwrap_err(), InboxError::TooManyPending);
}

fn create_input() -> CreateInput {
    CreateInput {
        kind: InboxType::Choice,
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
        level: None,
        source: None,
        blocking: true,
        dedupe_key: None,
        payload: wonton_protocol::InboxPayload::Choice(ChoicePayload {
            questions: vec![question()],
        }),
        expires_in_ms: None,
        page: None,
    }
}

/// 只列发给这个人的；别人的看不到。
#[test]
fn lists_are_per_recipient() {
    let (inbox, clock) = inbox();
    create(&inbox);
    assert_eq!(inbox.list("alice", clock.get()).items.len(), 0);
    assert_eq!(inbox.list("owner", clock.get()).items.len(), 1);
    assert!(
        inbox
            .view("alice", &inbox.list("owner", 0).items[0].id, clock.get())
            .is_err()
    );
}

/// 稍后提醒把条目挪出列表，不延长期限。
#[test]
fn snooze_removes_from_the_list() {
    let (inbox, clock) = inbox();
    let created = create(&inbox);
    inbox
        .snooze("owner", &created.item.id, InboxSnoozePreset::Hour, Some(0))
        .unwrap();
    assert!(inbox.list("owner", clock.get()).items.is_empty());
    clock.set(clock.get() + 3_600_000 + 1);
    assert_eq!(inbox.list("owner", clock.get()).items.len(), 1);
}
