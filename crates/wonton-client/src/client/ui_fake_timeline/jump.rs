//! 跳到被引用的那条、再按原路回来：找不到时的加载、放弃，和回来时的返回栈（ADR-0109 §4）。

use std::sync::Arc;

use wonton_protocol::{JumpState, TimelineContent, TimelineFocus};

use super::timeline::FakeTimeline;

/// 假核心找不到原消息时多久放弃。调大一点，测试才能靠“第二下有没有重新派”把时序分开。
const GIVE_UP: std::time::Duration = std::time::Duration::from_millis(2_000);

impl FakeTimeline {
    pub(super) fn register_revision(&self, revision: u32) {
        self.lock().revision = revision;
    }

    /// 找不到原消息时先报加载，停一会儿再说找不到（真核心这时在往前翻页）。期间壳又报了新序号就作废这一跳。
    /// 假核心找不到原消息时多久放弃。壳里“加载中/放弃”的时序只能靠它，测试也按它算等待上限。
    pub(super) fn give_up_later(self: &Arc<Self>, revision: u32) {
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(GIVE_UP);
            if this.lock().revision != revision {
                return;
            }
            this.batch(|update| update.jump = JumpState::Unavailable);
        });
    }

    pub(super) fn jump_to(self: &Arc<Self>, event_id: &str, from_item_id: String) {
        let target = self
            .lock()
            .items
            .iter()
            .find_map(|item| match &item.content {
                TimelineContent::Message { message }
                    if message.event_id.as_deref() == Some(event_id) =>
                {
                    Some(item.id.clone())
                }
                _ => None,
            });
        let Some(item_id) = target else {
            let revision = self.lock().revision;
            self.batch(|update| update.jump = JumpState::Loading);
            self.give_up_later(revision);
            return;
        };
        self.lock().back.push(from_item_id);
        self.batch(|update| {
            update.focus = Some(TimelineFocus {
                item_id,
                highlight: true,
            });
        });
    }

    pub(super) fn jump_back(&self) {
        let back = {
            let mut state = self.lock();
            let mut found = None;
            while let Some(item_id) = state.back.pop() {
                if state.items.iter().any(|item| item.id == item_id) {
                    found = Some(item_id);
                    break;
                }
            }
            found
        };
        self.batch(|update| match back {
            Some(item_id) => {
                update.focus = Some(TimelineFocus {
                    item_id,
                    highlight: false,
                });
            }
            None => update.scroll_to_bottom = true,
        });
    }
}
