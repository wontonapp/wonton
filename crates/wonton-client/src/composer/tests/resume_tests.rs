//! 发文件途中断网、家里的服务器出错（#647）：断了是暂停，连上接着传，不用主人点；主人点了重新上传，做不成时停在说得出下一步的状态上。

use super::*;
use wonton_protocol::FileFailure;

const REPORT: &str = "t-report.pdf";

fn file_state(fake: &Fake, id: &str, state: TransferState) {
    fake.transfers.send_modify(|view| {
        let transfer = view.active.iter_mut().find(|seen| seen.id == id).unwrap();
        transfer.state = state;
    });
}

fn outgoing_failed(setup: &Setup) -> bool {
    matches!(
        view(setup).outgoing.first().map(|outgoing| &outgoing.state),
        Some(SendState::Failed { .. })
    )
}

fn file_attachment(setup: &Setup) -> AttachmentState {
    view(setup).outgoing[0].attachments[0].state.clone()
}

/// 一条带文件和字的待发消息，文件已经开始传。
async fn sending_report() -> (Setup, String) {
    let s = setup();
    let attachment = s
        .link
        .add_file(ROOM, file_source("report.pdf"), "report.pdf".into(), None)
        .unwrap();
    s.link.send(ROOM, "看看这个", None, &[]).unwrap();
    until("文件开始上传", || {
        s.fake.transfers.borrow().active.len() == 1
    })
    .await;
    (s, attachment)
}

fn sent_with(s: &Setup, file: &FileId) {
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].parts,
        [Part::Text {
            body: format!("看看这个\n\n{}", file.link()),
            mentions: Vec::new()
        }]
    );
}

#[tokio::test]
async fn a_file_cut_off_midway_waits_then_sends_without_a_retry() {
    let (s, _) = sending_report().await;
    file_state(&s.fake, REPORT, TransferState::Paused);
    until("暂停，整条还在发", || {
        file_attachment(&s) == AttachmentState::Paused
            && matches!(view(&s).outgoing[0].state, SendState::Sending { .. })
    })
    .await;

    file_state(&s.fake, REPORT, TransferState::Running);
    let file = FileId::from_bytes([3; 16]);
    s.fake.finish_file(REPORT, &file);
    until("交出去了", || view(&s).outgoing.is_empty()).await;
    assert_eq!(
        *s.fake.file_calls.lock().unwrap(),
        ["report.pdf"],
        "接着传，不从头再传"
    );
    sent_with(&s, &file);
}

/// 家里的服务器忙、没建成上传（#647 实测的就是这种），主人点重新上传，这时又没网：附件是等连接，不是又一次失败；连上传完照常发。
#[tokio::test]
async fn retrying_without_a_connection_waits_for_it_then_sends() {
    let (s, attachment) = sending_report().await;
    file_state(
        &s.fake,
        REPORT,
        TransferState::Failed {
            reason: FileFailure::ServiceUnavailable,
            message: "busy".into(),
        },
    );
    until("整条失败", || outgoing_failed(&s)).await;

    s.link.retry(ROOM, &attachment).unwrap();
    until("重新开始上传", || {
        s.fake.file_calls.lock().unwrap().len() == 2
    })
    .await;
    let again = "t-report.pdf-2";
    file_state(&s.fake, again, TransferState::Paused);
    until("等连接", || {
        file_attachment(&s) == AttachmentState::Paused
    })
    .await;
    assert!(matches!(
        view(&s).outgoing[0].state,
        SendState::Sending { .. }
    ));

    let file = FileId::from_bytes([4; 16]);
    s.fake.finish_file(again, &file);
    until("交出去了", || view(&s).outgoing.is_empty()).await;
    sent_with(&s, &file);
}

/// 重新上传也读不了原文件（系统收回了读权限、文件被删）：停在“原文件读不了”上，壳据此请主人重新选，不说成服务器出错；
/// 字和后面的消息都不发。
#[tokio::test]
async fn a_retry_that_cannot_read_the_file_asks_to_choose_it_again() {
    let (s, attachment) = sending_report().await;
    file_state(
        &s.fake,
        REPORT,
        TransferState::Failed {
            reason: FileFailure::UpstreamError,
            message: "upstream".into(),
        },
    );
    until("整条失败", || outgoing_failed(&s)).await;

    s.link.retry(ROOM, &attachment).unwrap();
    until("重新开始上传", || {
        s.fake.file_calls.lock().unwrap().len() == 2
    })
    .await;
    file_state(
        &s.fake,
        "t-report.pdf-2",
        TransferState::Failed {
            reason: FileFailure::LocalSourceUnavailable,
            message: "gone".into(),
        },
    );
    until("停在原文件读不了", || {
        matches!(
            file_attachment(&s),
            AttachmentState::Failed {
                reason: AttachmentFailure::SourceUnavailable,
                ..
            }
        ) && outgoing_failed(&s)
    })
    .await;
    settle().await;
    assert_eq!(s.fake.groups(), 0);
}
