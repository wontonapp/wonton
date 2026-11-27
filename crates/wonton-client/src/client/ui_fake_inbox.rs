//! 假数据入口（`ui_fake.rs`）的收件箱：没有核心可取，照假的确认卡片列表现编一份核心列表，再走真实的合成，
//! 界面测试看到的条目、铃铛计数和真核心下同一个口径。

use tokio::sync::watch;
use wonton_protocol::{
    ConfirmationView, ConfirmationsViewModel, InboxCounts, InboxList, InboxListItem,
    InboxRecipient, InboxSender, InboxSenderKind, InboxStatus, InboxType, InboxViewModel,
};

use crate::chat::inbox::compose;

/// 跟着假卡片列表重算收件箱。假数据入口没有作用域的运行时可借，另起线程。
pub(super) fn follow(
    mut cards: watch::Receiver<ConfirmationsViewModel>,
    inbox: watch::Sender<InboxViewModel>,
) {
    let publish = |cards: &ConfirmationsViewModel| {
        inbox.send_replace(compose(Some(&list_of(cards)), cards, None));
    };
    publish(&cards.borrow_and_update());
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime always builds");
        runtime.block_on(async move {
            while cards.changed().await.is_ok() {
                let view = cards.borrow_and_update().clone();
                inbox.send_replace(compose(Some(&list_of(&view)), &view, None));
            }
        });
    });
}

fn list_of(cards: &ConfirmationsViewModel) -> InboxList {
    let item = |view: &ConfirmationView, status: InboxStatus| InboxListItem {
        id: format!("c-{}", view.card.id),
        kind: InboxType::Confirm,
        status,
        sender: InboxSender {
            kind: InboxSenderKind::Agent,
            id: "app.hermes".into(),
            display_name: "Hermes".into(),
            avatar: None,
        },
        recipient: InboxRecipient::Person {
            username: "owner".into(),
        },
        title: view.card.title.clone(),
        body: String::new(),
        level: None,
        created_at_ms: view.card.expires_at_ms.saturating_sub(600_000),
        expires_at_ms: Some(view.card.expires_at_ms),
        snooze_until_ms: None,
        needs_answer: status == InboxStatus::Pending,
        unread: status == InboxStatus::Pending,
        read_by: Vec::new(),
        first_question: None,
        question_count: 0,
        action_count: 0,
        progress: None,
    };
    let items: Vec<InboxListItem> = cards
        .pending
        .iter()
        .map(|view| item(view, InboxStatus::Pending))
        .chain(
            cards
                .finished
                .iter()
                .map(|view| item(view, InboxStatus::Handled)),
        )
        .collect();
    InboxList {
        counts: InboxCounts {
            needing: u32::try_from(cards.pending.len()).unwrap_or(u32::MAX),
            unread_notices: 0,
        },
        items,
        revision: 1,
    }
}
