//! 离底以后的后台来消息：未读数字只涨不清，回到底部清零。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use wonton_protocol::{
    TimelineContent, TimelineDiff, TimelineItem, TimelineScroll, TimelineViewport,
};

use super::timeline::FakeTimeline;
use super::ui_fake_room::{ECHO, ITEMS, at_ms, text_message};

/// 走查里翻走后后台陆续来的三条。
const ARRIVALS: [&str; 3] = ["First arrival", "Second arrival", "Third arrival"];

impl FakeTimeline {
    pub(super) fn scrolled(self: &Arc<Self>, scroll: TimelineScroll) {
        match scroll {
            TimelineScroll::Bottom => {
                self.lock().back.clear();
                self.away.store(false, Ordering::SeqCst);
                self.arriving.store(0, Ordering::SeqCst);
            }
            TimelineScroll::Away => {
                let first = !self.away.swap(true, Ordering::SeqCst);
                if first {
                    self.start_arrivals();
                }
            }
        }
        if scroll == TimelineScroll::Bottom {
            self.batch(|update| {
                update.viewport = wonton_protocol::TimelineViewport::Bottom;
            });
        }
    }

    /// 翻走以后后台陆续来三条，走查用：数字一条条涨，回到底部清掉，再翻走再排一轮。
    /// `WONTON_UI_FAKE_ARRIVALS` 改条数（看 99+ 角标要上百条），多于三条时挤在一秒多里来完。
    fn start_arrivals(self: &Arc<Self>) {
        if self.bursting.swap(true, Ordering::SeqCst) {
            return;
        }
        let total = std::env::var("WONTON_UI_FAKE_ARRIVALS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|total| *total > 0)
            .unwrap_or(ARRIVALS.len());
        let pause = if total > ARRIVALS.len() { 120 } else { 700 };
        let fake = Arc::downgrade(self);
        let arrivals = async move {
            for index in 0..total {
                tokio::time::sleep(Duration::from_millis(pause)).await;
                let Some(fake) = fake.upgrade() else { return };
                fake.arrive(ARRIVALS[index % ARRIVALS.len()]);
            }
            if let Some(fake) = fake.upgrade() {
                fake.bursting.store(false, Ordering::SeqCst);
            }
        };
        match &self.handle {
            Some(handle) => {
                handle.spawn(arrivals);
            }
            None => {
                tokio::spawn(arrivals);
            }
        }
    }

    /// 后台来的一条（回声 Bot 发的）。
    fn arrive(&self, body: &str) {
        let seq = self.arrived.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("ui-fake-arrived-{seq}");
        let message = text_message(
            &id,
            ECHO,
            at_ms(ITEMS + 100 + seq as usize),
            &[body.to_owned()],
        );
        let item = TimelineItem {
            id,
            content: TimelineContent::Message { message },
        };
        let viewport = if self.away.load(Ordering::SeqCst) {
            let index = self.arriving.fetch_add(1, Ordering::SeqCst) + 1;
            TimelineViewport::Away {
                new_messages: index,
            }
        } else {
            TimelineViewport::Bottom
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::PushBack { item }];
            update.viewport = viewport;
        });
    }
}

#[cfg(test)]
#[path = "unread_tests.rs"]
mod unread_tests;
