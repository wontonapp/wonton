//! 到设备的命令通道（ADR-0113 第 5 节，M13）：核心不直连手机，设备拉。
//!
//! 工具经 [`Commands::deliver`] 给某台设备排一条命令并等它回执，30 秒没回执算设备没回应；设备经
//! [`Commands::poll`] 长轮询取命令（最长挂 25 秒），做完经 [`Commands::ack`] 回执。命令落在媒体库里，
//! `seq` 每台设备单调、核心重启后接着走，设备按它说“我回执到哪了”。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{oneshot, watch};
use wonton_protocol::media::api::CommandAck;
use wonton_protocol::media::{DeviceCommand, MediaAction, MediaFailure, TaskId};

use super::store::{AckState, MediaStoreError, Store};

/// 长轮询最长挂多久。
pub const COMMAND_POLL: Duration = Duration::from_secs(25);
/// 等设备回执多久，过了算设备没回应。
pub const COMMAND_ACK: Duration = Duration::from_secs(30);

/// 设备对一条命令怎么说。
#[derive(Debug, Clone, PartialEq)]
pub enum CommandOutcome {
    Done,
    Failed(MediaFailure),
    /// 等满了也没回执。可能关机、没网、被划掉、被系统限流，核心分不出是哪一种。
    NoResponse,
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Store(#[from] MediaStoreError),
    #[error("命令 {0} 不是发给这台设备的")]
    NotThisDevice(TaskId),
    #[error("命令 {command} 的 seq 是 {expected}，回执写的是 {got}")]
    SeqMismatch {
        command: TaskId,
        expected: u64,
        got: u64,
    },
}

pub struct Commands {
    store: Arc<Store>,
    /// 有新命令就加一，长轮询挂在它上面。
    revision: watch::Sender<u64>,
    /// 等回执的工具，按命令号。
    waiting: Mutex<HashMap<TaskId, oneshot::Sender<CommandOutcome>>>,
}

impl Commands {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            revision: watch::channel(0).0,
            waiting: Mutex::new(HashMap::new()),
        }
    }

    /// 给 `device` 排一条命令，等它回执，最多等 `within`。
    ///
    /// 等满了就把还没回执的命令撤掉：工具已经告诉 Agent 设备没回应，设备之后再上线不该悄悄照做。
    pub async fn deliver(
        &self,
        device: &str,
        body: &MediaAction,
        within: Duration,
    ) -> Result<CommandOutcome, CommandError> {
        let (sender, answer) = oneshot::channel();
        // 先登记等待再放出去：设备的回执可能比这边走到等待更早到。
        let command = {
            let mut waiting = self.waiting.lock().expect("命令等待表锁中毒");
            let command = self.store.push_command(device, body)?;
            waiting.insert(command.command_id, sender);
            command
        };
        self.revision.send_modify(|revision| *revision += 1);
        if let Ok(Ok(outcome)) = tokio::time::timeout(within, answer).await {
            return Ok(outcome);
        }
        self.waiting
            .lock()
            .expect("命令等待表锁中毒")
            .remove(&command.command_id);
        self.store.withdraw_command(command.command_id)?;
        Ok(CommandOutcome::NoResponse)
    }

    /// 这台设备 `after` 之后还没回执的命令；没有就挂到有新命令或 [`COMMAND_POLL`]，到点还没有回空。
    /// 先订阅再查库，中间排进来的命令不会漏掉。
    pub async fn poll(&self, device: &str, after: u64) -> Result<Vec<DeviceCommand>, CommandError> {
        let mut watcher = self.revision.subscribe();
        let deadline = tokio::time::Instant::now() + COMMAND_POLL;
        loop {
            let commands = self.store.commands_after(device, after)?;
            if !commands.is_empty() {
                return Ok(commands);
            }
            match tokio::time::timeout_at(deadline, watcher.changed()).await {
                Ok(Ok(())) => {
                    watcher.borrow_and_update();
                }
                _ => return Ok(Vec::new()),
            }
        }
    }

    /// 设备的回执。重复的不再通知；回执里的 `seq` 要和这条命令的对得上。
    pub fn ack(&self, device: &str, command: TaskId, ack: &CommandAck) -> Result<(), CommandError> {
        match self.store.ack_command(device, command, ack.seq)? {
            AckState::First => {
                let waiter = self
                    .waiting
                    .lock()
                    .expect("命令等待表锁中毒")
                    .remove(&command);
                if let Some(waiter) = waiter {
                    // 等的那边刚好超时走了也没关系：回执照收，结果没人要了。
                    let _ = waiter.send(outcome(ack));
                }
                Ok(())
            }
            // 已经回执过的、撤掉或清掉了的都当重复：回执是幂等的。
            AckState::Repeat => Ok(()),
            AckState::OtherDevice => Err(CommandError::NotThisDevice(command)),
            AckState::SeqMismatch { expected } => Err(CommandError::SeqMismatch {
                command,
                expected,
                got: ack.seq,
            }),
        }
    }
}

fn outcome(ack: &CommandAck) -> CommandOutcome {
    match (&ack.ok, &ack.failure) {
        (true, _) => CommandOutcome::Done,
        (false, Some(failure)) => CommandOutcome::Failed(failure.clone()),
        (false, None) => CommandOutcome::Failed(MediaFailure {
            code: wonton_protocol::media::MediaFailureCode::Protocol,
            message: "设备说没做成，没带原因".into(),
            retry_after_secs: None,
        }),
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
