//! 解不开的加密消息：先当“密钥晚到”等，超过时限还没解开就报一次，不静默（ADR-0035 失败即停）。
//!
//! crypto store 丢了或坏了时，Bot 解不开收到的每一条加密消息；桥按“密钥可能晚于事件到达”把它们
//! 跳过，主人只看到 Bot 不回话。这里把等得过久的事件记下来，到点交给房间循环去发提示、标健康。

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use wonton_matrix::{Message, MessageBody, OwnedEventId, OwnedUserId};

/// 还没解开的事件：按事件 ID 记第一次见到的时间；发过提示的时间单独记，别刷屏。
#[derive(Default)]
pub(crate) struct Tracker {
    waiting: HashMap<OwnedEventId, Instant>,
    last_notice: Option<Instant>,
}

impl Tracker {
    /// 每批新到的消息记一笔（启动时的历史不算，见 `run_room`）：解不开的开始等，解开了的移出。
    /// 只认名单里的人：没资格跟 Bot 说话的人的消息解不开也不用报。
    pub(crate) fn observe(&mut self, messages: &[Message], allowed: &[OwnedUserId]) {
        for message in messages {
            let Some(event_id) = message.event_id.as_ref() else {
                continue;
            };
            let unreadable = matches!(message.body, MessageBody::Undecryptable)
                && allowed.contains(&message.sender);
            if unreadable {
                self.waiting
                    .entry(event_id.clone())
                    .or_insert_with(Instant::now);
            } else {
                self.waiting.remove(event_id);
            }
        }
    }

    /// 有事件等了 `after` 还没解开就算“解不开”。到了 `every` 的间隔才允许再说一条，别刷屏；
    /// 返回要不要在这个房间发提示。判定过的事件留在名单里：解开时由 [`Self::observe`] 移出。
    pub(crate) fn report_due(&mut self, after: Duration, every: Duration, now: Instant) -> bool {
        if !self
            .waiting
            .values()
            .any(|seen| now.duration_since(*seen) >= after)
        {
            return false;
        }
        if let Some(last) = self.last_notice
            && now.duration_since(last) < every
        {
            return false;
        }
        self.last_notice = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_matrix::OwnedUserId;

    fn owner() -> OwnedUserId {
        "@owner:example.org".parse().unwrap()
    }

    fn stranger() -> OwnedUserId {
        "@stranger:example.org".parse().unwrap()
    }

    fn message(sender: &OwnedUserId, event_id: &str, undecryptable: bool) -> Message {
        Message {
            event_id: Some(event_id.parse().unwrap()),
            sender: sender.clone(),
            sender_name: String::new(),
            is_own: false,
            timestamp_ms: 1,
            status: wonton_matrix::SendStatus::Sent,
            body: if undecryptable {
                MessageBody::Undecryptable
            } else {
                MessageBody::Text("解开了".into())
            },
            custom: Default::default(),
            mentions: Vec::new(),
            thread_root: None,
            edited: false,
            read_by: Vec::new(),
            reply: None,
            agent_run: None,
            group: None,
            can_redact: false,
        }
    }

    #[test]
    fn a_message_still_undecryptable_after_the_limit_is_due() {
        let mut tracker = Tracker::default();
        let allowed = [owner()];
        tracker.observe(&[message(&owner(), "$a", true)], &allowed);
        let start = Instant::now();
        assert!(!tracker.report_due(
            Duration::from_secs(60),
            Duration::from_secs(600),
            start + Duration::from_secs(59)
        ));
        assert!(tracker.report_due(
            Duration::from_secs(60),
            Duration::from_secs(600),
            start + Duration::from_secs(60)
        ));
    }

    #[test]
    fn a_message_that_decrypts_in_time_is_never_reported() {
        let mut tracker = Tracker::default();
        let allowed = [owner()];
        tracker.observe(&[message(&owner(), "$a", true)], &allowed);
        let start = Instant::now();
        tracker.observe(&[message(&owner(), "$a", false)], &allowed);
        assert!(!tracker.report_due(
            Duration::from_secs(60),
            Duration::from_secs(600),
            start + Duration::from_secs(120)
        ));
    }

    #[test]
    fn the_notice_repeats_only_after_the_interval() {
        let mut tracker = Tracker::default();
        let allowed = [owner()];
        tracker.observe(&[message(&owner(), "$a", true)], &allowed);
        let start = Instant::now();
        let every = Duration::from_secs(600);
        assert!(tracker.report_due(
            Duration::from_secs(60),
            every,
            start + Duration::from_secs(60)
        ));
        // 同一条还没解开：间隔内不再说，过了间隔再说。
        assert!(!tracker.report_due(every, every, start + Duration::from_secs(659)));
        assert!(tracker.report_due(every, every, start + Duration::from_secs(660)));
    }

    #[test]
    fn strangers_undecryptable_messages_are_ignored() {
        let mut tracker = Tracker::default();
        let allowed = [owner()];
        tracker.observe(&[message(&stranger(), "$a", true)], &allowed);
        let start = Instant::now();
        assert!(!tracker.report_due(
            Duration::from_secs(60),
            Duration::from_secs(600),
            start + Duration::from_secs(3600)
        ));
    }
}
