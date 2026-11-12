//! 时间线里不显示的条目（ADR-0093 §4）：主对话里同一个旁聊只留最新一条结果行，旁聊时间线里带标记的根不作为条目
//! （标题和状态放在订阅的头部）。
//!
//! SDK 的时间线照样带着它们（下标、回执、翻页都按 SDK 的来），这里在 SDK 的条目和交给调用方的条目之间换算：
//! 每条增量按换算前后各自显示不显示，翻成调用方那一侧的增删改；一批应用完以后再重算哪些该显示（新的结果行会让
//! 旧的那条藏起来），变了的补一条插入或删除。

use futures_util::{Stream, StreamExt, stream};
use matrix_sdk_ui::eyeball_im::{Vector, VectorDiff};

use super::Focus;
use crate::side_chat::{result_line, root_marker};
use crate::timeline::{EntryKind, TimelineEntry};

/// SDK 的整条时间线和每条显示不显示。
struct Visible {
    focus: Focus,
    full: Vector<TimelineEntry>,
    shown: Vec<bool>,
}

pub(super) fn project(
    focus: Focus,
    entries: Vec<TimelineEntry>,
    diffs: impl Stream<Item = Vec<VectorDiff<TimelineEntry>>> + Unpin,
) -> (
    Vec<TimelineEntry>,
    impl Stream<Item = Vec<VectorDiff<TimelineEntry>>> + Unpin,
) {
    let mut visible = Visible {
        focus,
        full: entries.into_iter().collect(),
        shown: Vec::new(),
    };
    visible.shown = visible.wanted();
    let current = visible.visible();
    let updates = stream::unfold((visible, diffs), |(mut visible, mut diffs)| async move {
        loop {
            let batch = diffs.next().await?;
            let out = visible.apply(batch);
            if !out.is_empty() {
                return Some((out, (visible, diffs)));
            }
        }
    });
    (current, Box::pin(updates))
}

impl Visible {
    fn visible(&self) -> Vec<TimelineEntry> {
        self.full
            .iter()
            .zip(&self.shown)
            .filter(|(_, shown)| **shown)
            .map(|(entry, _)| entry.clone())
            .collect()
    }

    /// 调用方那一侧的下标：SDK 下标之前显示着的条数。
    fn at(&self, index: usize) -> usize {
        self.shown[..index].iter().filter(|shown| **shown).count()
    }

    /// 只看这一条自己就知道不显示的：旁聊时间线里带标记的根。
    fn alone(&self, entry: &TimelineEntry) -> bool {
        let Focus::Thread(root) = &self.focus else {
            return true;
        };
        match &entry.kind {
            EntryKind::Message(message) => {
                message.event_id.as_ref() != Some(root) || root_marker(&message.custom).is_none()
            }
            _ => true,
        }
    }

    /// 每条该不该显示：除了 [`Self::alone`]，主对话里每个旁聊只留最后一条结果行。
    fn wanted(&self) -> Vec<bool> {
        let mut wanted: Vec<bool> = self.full.iter().map(|entry| self.alone(entry)).collect();
        if self.focus != Focus::Main {
            return wanted;
        }
        let mut latest = std::collections::HashSet::new();
        for (index, entry) in self.full.iter().enumerate().rev() {
            let EntryKind::Message(message) = &entry.kind else {
                continue;
            };
            if let Some(line) = result_line(&message.custom, 0)
                && !latest.insert(line.root)
            {
                wanted[index] = false;
            }
        }
        wanted
    }

    fn apply(&mut self, batch: Vec<VectorDiff<TimelineEntry>>) -> Vec<VectorDiff<TimelineEntry>> {
        let mut out = Vec::new();
        for diff in batch {
            self.translate(diff, &mut out);
        }
        self.reconcile(&mut out);
        out
    }

    fn translate(
        &mut self,
        diff: VectorDiff<TimelineEntry>,
        out: &mut Vec<VectorDiff<TimelineEntry>>,
    ) {
        match diff {
            VectorDiff::Append { values } => {
                let mut shown = Vector::new();
                for value in values {
                    let flag = self.alone(&value);
                    if flag {
                        shown.push_back(value.clone());
                    }
                    self.full.push_back(value);
                    self.shown.push(flag);
                }
                if !shown.is_empty() {
                    out.push(VectorDiff::Append { values: shown });
                }
            }
            VectorDiff::Clear => {
                self.full.clear();
                self.shown.clear();
                out.push(VectorDiff::Clear);
            }
            VectorDiff::PushFront { value } => self.insert(0, value, out),
            VectorDiff::PushBack { value } => self.insert(self.full.len(), value, out),
            VectorDiff::PopFront => self.remove(0, out),
            VectorDiff::PopBack => {
                if let Some(last) = self.full.len().checked_sub(1) {
                    self.remove(last, out);
                }
            }
            VectorDiff::Insert { index, value } => self.insert(index, value, out),
            VectorDiff::Remove { index } => self.remove(index, out),
            VectorDiff::Set { index, value } => self.set(index, value, out),
            VectorDiff::Truncate { length } => {
                let before = self.at(self.shown.len());
                let after = self.at(length.min(self.shown.len()));
                self.full.truncate(length);
                self.shown.truncate(length);
                if after != before {
                    out.push(VectorDiff::Truncate { length: after });
                }
            }
            VectorDiff::Reset { values } => {
                self.full = values;
                self.shown = self.wanted();
                out.push(VectorDiff::Reset {
                    values: self.visible().into_iter().collect(),
                });
            }
        }
    }

    fn insert(
        &mut self,
        index: usize,
        value: TimelineEntry,
        out: &mut Vec<VectorDiff<TimelineEntry>>,
    ) {
        let flag = self.alone(&value);
        let at = self.at(index);
        if flag {
            out.push(VectorDiff::Insert {
                index: at,
                value: value.clone(),
            });
        }
        self.full.insert(index, value);
        self.shown.insert(index, flag);
    }

    fn remove(&mut self, index: usize, out: &mut Vec<VectorDiff<TimelineEntry>>) {
        if index >= self.full.len() {
            return;
        }
        if self.shown[index] {
            out.push(VectorDiff::Remove {
                index: self.at(index),
            });
        }
        self.full.remove(index);
        self.shown.remove(index);
    }

    fn set(
        &mut self,
        index: usize,
        value: TimelineEntry,
        out: &mut Vec<VectorDiff<TimelineEntry>>,
    ) {
        if index >= self.full.len() {
            return;
        }
        let at = self.at(index);
        let (was, now) = (self.shown[index], self.alone(&value));
        match (was, now) {
            (true, true) => out.push(VectorDiff::Set {
                index: at,
                value: value.clone(),
            }),
            (true, false) => out.push(VectorDiff::Remove { index: at }),
            (false, true) => out.push(VectorDiff::Insert {
                index: at,
                value: value.clone(),
            }),
            (false, false) => {}
        }
        self.full.set(index, value);
        self.shown[index] = now;
    }

    /// 一批应用完以后重算：新来的结果行让同一个旁聊更早的那条藏起来，删掉最新一条时上一条重新露出来。
    fn reconcile(&mut self, out: &mut Vec<VectorDiff<TimelineEntry>>) {
        let wanted = self.wanted();
        for (index, want) in wanted.into_iter().enumerate() {
            if self.shown[index] == want {
                continue;
            }
            let at = self.at(index);
            if want {
                out.push(VectorDiff::Insert {
                    index: at,
                    value: self.full[index].clone(),
                });
            } else {
                out.push(VectorDiff::Remove { index: at });
            }
            self.shown[index] = want;
        }
    }
}

#[cfg(test)]
#[path = "visible_tests.rs"]
mod tests;
