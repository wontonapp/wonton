//! 一条时间线的编辑态（ADR-0109 第 10 节）：正在改哪条、原草稿是什么。
//!
//! 编辑态只在本机：App 重启后就没了，草稿里留着要改的正文。原草稿在进入编辑态时存下来，取消和发送成功后原样恢复；编辑途中又打了字就以新打的为准。

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;
use wonton_matrix::Draft;
use wonton_protocol::ComposerEdit;

use super::util::lock;

/// 原正文摘要最多这么多个字，编辑条一行放得下。
const SUMMARY_CHARS: usize = 80;

/// 一条时间线的编辑态。
#[derive(Default)]
pub(super) struct Edit {
    active: Mutex<Option<Active>>,
    /// 每次请求换一个号：异步读草稿期间又开始了别的（或取消了），旧的不再生效。
    token: AtomicU64,
    /// 编辑态变过的次数，转发循环按它补发一批。
    changed: watch::Sender<u64>,
    /// 上一批发出去的编辑态，一样的不重复发。
    last_sent: Mutex<Option<Option<ComposerEdit>>>,
}

struct Active {
    item_id: String,
    event_id: String,
    summary: String,
    /// 进编辑态时放进输入框的原文：退出时输入框里还是它，才拿原草稿盖回去。
    body: String,
    /// 进入编辑态之前输入框里的草稿，取消或发送成功后原样恢复。
    saved: Draft,
}

impl Edit {
    /// 领一个号给这次 `StartEdit`。
    pub(super) fn begin_token(&self) -> u64 {
        self.token.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// 换掉编辑目标；号对不上（期间又开始了别的或取消了）返回假。
    ///
    /// `write` 在编辑态已经换上、但 `active` 还锁着时调用：取消要等它写完，草稿队列里“编辑正文”“恢复原草稿”
    /// 的先后和这两次状态切换一致。
    pub(super) fn begin(
        &self,
        token: u64,
        item_id: String,
        event_id: String,
        text: &str,
        saved: Draft,
        write: impl FnOnce(Draft),
    ) -> bool {
        let body = text.to_owned();
        let active = Active {
            item_id,
            event_id,
            summary: summary(text),
            body: body.clone(),
            saved,
        };
        // 号和状态在同一把锁下核对：取消是“先换号、再拿状态”，先拿锁的那一方赢。
        let mut slot = lock(&self.active);
        if self.token.load(Ordering::SeqCst) != token {
            return false;
        }
        *slot = Some(active);
        write(Draft {
            body,
            reply_to: None,
        });
        drop(slot);
        self.wake();
        true
    }

    /// 退出编辑态并拿走要发的事件 ID 和原草稿；没在编辑时为空。
    pub(super) fn take(&self) -> Option<(String, String, Draft)> {
        self.token.fetch_add(1, Ordering::SeqCst);
        let active = lock(&self.active).take()?;
        self.wake();
        Some((active.event_id, active.body, active.saved))
    }

    /// 进入编辑态之前的那份草稿。
    pub(super) fn saved(&self) -> Option<Draft> {
        lock(&self.active)
            .as_ref()
            .map(|active| active.saved.clone())
    }

    /// 正在编辑的是不是这条事件。
    pub(super) fn is_editing(&self, event_id: &str) -> bool {
        lock(&self.active)
            .as_ref()
            .is_some_and(|active| active.event_id == event_id)
    }

    /// 现在的编辑态，每一批都带当前值。
    pub(super) fn card(&self) -> Option<ComposerEdit> {
        lock(&self.active).as_ref().map(|active| ComposerEdit {
            item_id: active.item_id.clone(),
            event_id: active.event_id.clone(),
            summary: active.summary.clone(),
        })
    }

    /// 和上一批不一样时 `true` 并记下这一份：一样的编辑态不必补发一批。
    pub(super) fn take_changed(&self) -> bool {
        let card = self.card();
        if *lock(&self.last_sent) == Some(card.clone()) {
            return false;
        }
        *lock(&self.last_sent) = Some(card);
        true
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn wake(&self) {
        self.changed.send_modify(|value| *value += 1);
    }
}

/// 编辑条上的一行摘要：第一个非空行，去掉首尾空白，太长截断。
fn summary(text: &str) -> String {
    let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return String::new();
    };
    if line.chars().count() <= SUMMARY_CHARS {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(SUMMARY_CHARS).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod edit_tests;
