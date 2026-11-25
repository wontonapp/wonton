//! 收件箱行为测试：给定操作序列，断言 ViewModel 和对外副作用（ADR-0119 第 3 节）。

use std::sync::Arc;

use wonton_protocol::{
    ChoicePayload, InboxAnswerAction, InboxAnswerInput, InboxOption, InboxQuestion,
    InboxQuestionAnswer, InboxRecipient, InboxSender, InboxSenderKind, InboxSnoozePreset,
    InboxStatus, InboxType, PagePayload,
};

use super::NewPage;
use super::page::FixedClock;
use super::store::{CreateInput, Inbox, InboxError};
use super::{AdminSet, SystemClock};

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

fn page_input(html: &[u8], csp: &str, key: &str) -> CreateInput {
    CreateInput {
        kind: InboxType::Page,
        sender: InboxSender {
            kind: InboxSenderKind::Agent,
            id: "app.hermes".into(),
            display_name: "Hermes".into(),
            avatar: None,
        },
        recipient: InboxRecipient::Person {
            username: "owner".into(),
        },
        title: "报告".into(),
        body: String::new(),
        level: None,
        source: None,
        blocking: false,
        dedupe_key: Some(key.into()),
        payload: wonton_protocol::InboxPayload::Page(PagePayload {
            resource_uri: Some("ui://report".into()),
            file_id: None,
            caption: "报告".into(),
            network_domains: Vec::new(),
            actions: Vec::new(),
        }),
        expires_in_ms: None,
        page: Some(NewPage {
            html: html.to_vec(),
            csp: csp.into(),
        }),
    }
}

/// dedupe 命中要换新页面和 CSP，不是留着第一次的。
#[test]
fn dedupe_replaces_the_page_snapshot() {
    let (inbox, _clock) = inbox();
    let page = |html: &[u8], csp: &str, key: &str| page_input(html, csp, key);
    let first = inbox.create(page(b"<p>1</p>", "csp-1", "same")).unwrap();
    let second = inbox.create(page(b"<p>2</p>", "csp-2", "same")).unwrap();
    assert_eq!(
        first.item.id, second.item.id,
        "同一个 dedupe 键要更新同一条"
    );
    let (_html, csp) = inbox.page_bytes(&second.item.id).unwrap().unwrap();
    assert_eq!(csp, "csp-2");
    assert_eq!(
        inbox.page_bytes(&second.item.id).unwrap().unwrap().0,
        b"<p>2</p>"
    );
}

/// 过期后页面还留 30 天；30 天后 JSON 和 HTML 一起删。
#[test]
fn expired_pages_survive_until_cleanup() {
    let (inbox, clock) = inbox();
    let created = inbox
        .create(CreateInput {
            kind: InboxType::Page,
            sender: InboxSender {
                kind: InboxSenderKind::Agent,
                id: "app.hermes".into(),
                display_name: "Hermes".into(),
                avatar: None,
            },
            recipient: InboxRecipient::Person {
                username: "owner".into(),
            },
            title: "报告".into(),
            body: String::new(),
            level: None,
            source: None,
            blocking: false,
            dedupe_key: None,
            payload: wonton_protocol::InboxPayload::Page(PagePayload {
                resource_uri: Some("ui://report".into()),
                file_id: None,
                caption: "报告".into(),
                network_domains: Vec::new(),
                actions: Vec::new(),
            }),
            expires_in_ms: Some(1000),
            page: Some(NewPage {
                html: b"<p>ok</p>".to_vec(),
                csp: "csp".into(),
            }),
        })
        .unwrap();
    clock.set(clock.get() + 2000);
    inbox.sweep(clock.get());
    assert!(
        inbox.page_bytes(&created.item.id).unwrap().is_some(),
        "过期后还能看"
    );
    clock.set(clock.get() + 31 * 24 * 3600 * 1000);
    inbox.sweep(clock.get());
    assert!(
        inbox.page_bytes(&created.item.id).is_err(),
        "30 天后连页面一起删"
    );
}

/// 上次崩溃留下的临时文件不是条目：open 不读回，还顺手清掉。
#[test]
fn open_ignores_leftover_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_owned();
    let inbox = Inbox::for_test(&path);
    let created = create(&inbox);
    let item_path = path.join("items").join(format!("{}.json", created.item.id));
    let temp_path = path
        .join("items")
        .join(format!("{}.json.tmp.999", created.item.id));
    std::fs::copy(&item_path, &temp_path).unwrap();
    let page_path = path.join("pages").join(format!("{}.html", created.item.id));
    let page_tmp = path
        .join("pages")
        .join(format!("{}.html.tmp.999", created.item.id));
    std::fs::write(&page_path, b"<p>ok</p>").unwrap();
    std::fs::copy(&page_path, &page_tmp).unwrap();

    let reopened = Inbox::open(
        path.clone(),
        std::sync::Arc::new(SystemClock),
        std::sync::Arc::new(AdminSet::new("owner".into(), Vec::new())),
    )
    .unwrap();
    let list = reopened.list("owner", crate::inbox::now_ms());
    assert_eq!(list.items.len(), 1, "残留临时文件不能变成第二条");
    assert!(!temp_path.exists(), "条目残留临时文件要清掉");
    assert!(!page_tmp.exists(), "页面残留临时文件要清掉");
    assert!(page_path.exists(), "真正的页面要留着");
}

/// 并发 dedupe 同一条目：16 线程都走 create→write_page，读者在写的同时读，
/// 任何时刻都必须是完整的一份（write_private 的 truncate 窗口会读到 0 字节/半截）。
#[test]
fn concurrent_dedupe_pages_stay_whole() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    const LEN: usize = 256 * 1024;
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::for_test(dir.path());
    let first = inbox
        .create(page_input(&vec![b'z'; LEN], "csp", "same"))
        .unwrap();
    let id = first.item.id;
    for _round in 0..20 {
        let stop = Arc::new(AtomicBool::new(false));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let inbox = inbox.clone();
                let id = id.clone();
                let stop = stop.clone();
                scope.spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let (html, _csp) = inbox.page_bytes(&id).unwrap().unwrap();
                        assert_eq!(html.len(), LEN, "并发写把页面截断了");
                        assert!(
                            html.iter().all(|byte| *byte == html[0]),
                            "页面混进了另一份的字节"
                        );
                    }
                });
            }
            let mut writers = Vec::new();
            for i in 0..32u8 {
                let inbox = inbox.clone();
                writers.push(scope.spawn(move || {
                    let body = vec![b'a' + i; LEN];
                    inbox
                        .create(page_input(&body, &format!("csp-{i}"), "same"))
                        .unwrap();
                }));
            }
            // 等写线程都结束，再停读者。
            for writer in writers {
                writer.join().unwrap();
            }
            stop.store(true, Ordering::Relaxed);
        });
        let (html, _csp) = inbox.page_bytes(&id).unwrap().unwrap();
        assert_eq!(html.len(), LEN);
        assert!(html.iter().all(|byte| *byte == html[0]));
    }
}
