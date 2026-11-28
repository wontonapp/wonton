//! 命令通道在核心这一侧的行为：`seq` 每台设备单调、重启后接着走；等不到回执算没回应并撤掉命令。

use std::sync::Arc;

use wonton_protocol::media::{MediaAction, TaskId};

use super::{COMMAND_ACK, CommandOutcome, Commands};
use crate::media::store::Store;

const PHONE: &str = "phone";
const TABLET: &str = "tablet";

fn start(task: u128) -> MediaAction {
    MediaAction::StartDownload {
        task: TaskId::from_u128(task),
    }
}

#[test]
fn each_device_counts_its_own_seq_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.push_command(PHONE, &start(1)).unwrap().seq, 1);
    assert_eq!(store.push_command(PHONE, &start(2)).unwrap().seq, 2);
    assert_eq!(store.push_command(TABLET, &start(3)).unwrap().seq, 1);
    // 设备回执到 2，命令清掉了；重启后 `seq` 也不回到 1，否则设备会把新命令当成回执过的跳过。
    assert!(store.commands_after(PHONE, 2).unwrap().is_empty());
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    let next = store.push_command(PHONE, &start(4)).unwrap();
    assert_eq!(next.seq, 3);
    assert_eq!(store.commands_after(PHONE, 2).unwrap(), vec![next]);
}

#[tokio::test(start_paused = true)]
async fn a_device_that_never_answers_is_no_response_and_the_command_is_withdrawn() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    let commands = Commands::new(store.clone());
    let outcome = commands
        .deliver(PHONE, &start(1), COMMAND_ACK)
        .await
        .unwrap();
    assert_eq!(outcome, CommandOutcome::NoResponse);
    // 撤掉了：设备之后上线拉不到它。
    assert!(store.commands_after(PHONE, 0).unwrap().is_empty());
}
