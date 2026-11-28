use super::super::lock;
use super::super::timeline::TimelineSubscription;

impl TimelineSubscription {
    /// 跳到原消息：有就下一批给焦点，没有就往前翻页。
    pub(in crate::chat) fn jump_to(&self, event_id: &str, from_item_id: &str, revision: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps
                .request(event_id.to_owned(), from_item_id.to_owned(), revision);
        }
    }

    /// 胶囊：跳到最早未读（ADR-0121）。`position` 是读位置，`unread` 是房间未读数。
    pub(in crate::chat) fn jump_to_first_unread(&self, position: Option<String>, unread: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.request_first_unread(position, unread);
        }
    }

    /// 本会话冻结的读位置条目 ID（ADR-0121）。
    pub(in crate::chat) fn unread_read_item(&self) -> Option<String> {
        lock(&self.live)
            .as_ref()
            .and_then(|live| live.jumps.read_item())
    }

    /// 胶囊：跳到下一条未读 @（ADR-0121）。
    pub(in crate::chat) fn jump_to_next_mention(&self, position: Option<String>, unread: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.request_next_mention(position, unread);
        }
    }

    /// 壳报告锚点进视野：收掉胶囊。
    pub(in crate::chat) fn unread_anchor_seen(&self, item_id: String) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.anchor_seen(item_id);
        }
    }
    /// 当前报过的跳转序号。
    pub(in crate::chat) fn jumps_revision(&self) -> u32 {
        lock(&self.live)
            .as_ref()
            .map_or(0, |live| live.jumps.revision())
    }

    /// 壳报的跳转序号（[`wonton_protocol::TimelineAction::RegisterFocusRevision`]）。
    pub(in crate::chat) fn register_focus_revision(&self, revision: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.register_revision(revision);
        }
    }

    /// 回到跳之前的那条；栈空了就回到底部。
    pub(in crate::chat) fn jump_back(&self) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.request_back();
        }
    }
}
