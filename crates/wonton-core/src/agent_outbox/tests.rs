//! 出箱的行为测试：取走、重发、回报和落盘。

use std::sync::Arc;

use wonton_protocol::{BotRef, OutboxFailure, OutboxKind, OutboxReceipt, OutboxResult};

use super::Outbox;

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: "app.example.agent".into(),
        bot: name.into(),
    }
}

fn message(recipient: &str, body: &str) -> OutboxKind {
    OutboxKind::Message {
        recipient: recipient.into(),
        body: body.into(),
    }
}

/// 取走还没回报的 Message 项优先重发：同一项再次轮询还拿到它，后面的项排队等。
#[tokio::test]
async fn a_taken_message_is_redelivered_before_new_items() {
    let outbox = Outbox::default();
    let first = outbox.push(&bot("main"), message("mom", "第一条")).unwrap();
    let second = outbox.push(&bot("main"), message("mom", "第二条")).unwrap();

    let taken = outbox.next(&bot("main")).await.unwrap();
    assert_eq!(taken.id, first);
    // 桥没回报：再轮询还是同一项（重发靠事务 ID 去重），不会跳到第二条。
    let again = outbox.next(&bot("main")).await.unwrap();
    assert_eq!(again.id, first);

    outbox
        .receipt(&OutboxReceipt {
            id: first.clone(),
            result: OutboxResult::Sent {
                event_id: "$e1".into(),
            },
        })
        .then(|| ())
        .expect("回报认这项");
    let next = outbox.next(&bot("main")).await.unwrap();
    assert_eq!(next.id, second);
}

/// 回报说没做成也照实结清：项不再重发，失败类别留在回报里给核心审计。
#[tokio::test]
async fn a_failed_receipt_settles_the_item() {
    let outbox = Outbox::default();
    let id = outbox
        .push(&bot("main"), message("stranger", "在吗"))
        .unwrap();
    assert!(outbox.next(&bot("main")).await.is_some());
    outbox
        .receipt(&OutboxReceipt {
            id: id.clone(),
            result: OutboxResult::Failed {
                category: OutboxFailure::NoRoom,
            },
        })
        .then(|| ())
        .expect("回报认这项");
    assert!(
        outbox.next(&bot("main")).await.is_none(),
        "失败的项不再重发"
    );
}

/// 别的 Bot 取不走这个 Bot 的项；CancelTurn 不重发也不落盘。
#[tokio::test]
async fn each_bot_only_takes_its_own_items() {
    let outbox = Outbox::default();
    outbox
        .push(
            &bot("main"),
            OutboxKind::CancelTurn {
                chain: "c-1".into(),
                session: "s-1".into(),
            },
        )
        .unwrap();
    assert!(outbox.next(&bot("other")).await.is_none());
    let item = outbox.next(&bot("main")).await.unwrap();
    assert!(matches!(item.kind, OutboxKind::CancelTurn { .. }));
    assert!(outbox.next(&bot("main")).await.is_none(), "取消轮次不重发");
}

/// Message 项落盘：重启（重新打开）后照常交给桥，序号接着走，回报后从存档里清掉。
#[tokio::test]
async fn message_items_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent-outbox.json");
    let id = {
        let outbox = Outbox::with_store(&path).unwrap();
        let id = outbox
            .push(&bot("main"), message("mom", "周末回来吗"))
            .unwrap();
        // 取走不回报就退出：存档里的项重启后重交。
        assert!(outbox.next(&bot("main")).await.is_some());
        id
    };
    {
        let outbox = Outbox::with_store(&path).unwrap();
        let item = outbox.next(&bot("main")).await.unwrap();
        assert_eq!(item.id, id, "重启后同一项再交一次");
        outbox
            .receipt(&OutboxReceipt {
                id: id.clone(),
                result: OutboxResult::Sent {
                    event_id: "$e1".into(),
                },
            })
            .then(|| ())
            .expect("回报认这项");
    }
    let outbox = Outbox::with_store(&path).unwrap();
    assert!(
        outbox.next(&bot("main")).await.is_none(),
        "回报过的项清掉了"
    );
    // 序号接着存档里最大的走：重启后不开重号。
    let next = outbox.push(&bot("main"), message("mom", "又一条")).unwrap();
    assert_ne!(next, id);
    let number = |id: &str| id.strip_prefix("outbox-").unwrap().parse::<u64>().unwrap();
    assert!(number(&next) > number(&id));
}

/// 存档写不进去时入队没有发生：调用方拿到错误，不假装排上了。
#[tokio::test]
async fn a_failed_store_write_fails_the_push() {
    let dir = tempfile::tempdir().unwrap();
    // 父目录不存在：打开读得到空，写必失败。
    let outbox = Outbox::with_store(dir.path().join("missing/outbox.json")).unwrap();
    assert!(outbox.push(&bot("main"), message("mom", "第一条")).is_err());
    assert!(outbox.next(&bot("main")).await.is_none(), "写失败就没有项");
}

/// 回报结清的 Message 项回调在锁外逐条带上结果，给审计用。
#[tokio::test]
async fn settled_messages_reach_the_handler() {
    let outbox = Arc::new(Outbox::default());
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    outbox.on_message_settled(Arc::new(move |settled| {
        sink.lock().unwrap().push((
            settled.bot.bot.clone(),
            settled.recipient.clone(),
            settled.result.clone(),
        ));
    }));
    let id = outbox
        .push(&bot("main"), message("mom", "周末回来吗"))
        .unwrap();
    assert!(outbox.next(&bot("main")).await.is_some());
    outbox
        .receipt(&OutboxReceipt {
            id,
            result: OutboxResult::Sent {
                event_id: "$e1".into(),
            },
        })
        .then(|| ())
        .expect("回报认这项");
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(
            "main".into(),
            "mom".into(),
            OutboxResult::Sent {
                event_id: "$e1".into()
            },
        )]
    );
}
