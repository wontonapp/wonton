use std::sync::Arc;

use tokio::runtime::Handle;
use wonton_matrix::{ChatRoom, TimelineEntry, Vector};
use wonton_protocol::{JumpState, TimelineFocus, UnreadJumpState, UnreadJumpUnavailable};

use super::super::lock;
use super::Jumps;
use super::resolve::{Target, find, resolve};
use super::state::{Inner, Request, Wanted};

impl Jumps {
    /// 处理掉挂着的请求；有没有请求不管，结果都写进下一批。
    pub(super) fn begin(&self, entries: &Vector<TimelineEntry>) {
        let request = lock(&self.inner).request.take();
        match request {
            None => {}
            Some(Request::Jump {
                event,
                from,
                revision,
            }) => match find(entries, &event) {
                Some(item_id) => {
                    let mut inner = lock(&self.inner);
                    inner.revision = revision;
                    inner.back.push(from);
                    inner.focus = Some(TimelineFocus {
                        item_id,
                        highlight: true,
                    });
                    inner.state = JumpState::Idle;
                    inner.wanted = None;
                }
                None => {
                    let mut inner = lock(&self.inner);
                    inner.state = JumpState::Loading;
                    inner.wanted = Some(Wanted::Event { event, from });
                }
            },
            Some(Request::FirstUnread { position, unread }) => {
                self.begin_unread(entries, position, unread, false);
            }
            Some(Request::NextMention { position, unread }) => {
                self.begin_unread(entries, position, unread, true);
            }
            Some(Request::Seen { item_id }) => {
                let mut inner = lock(&self.inner);
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.unread_state = UnreadJumpState::Idle;
                inner.dismiss_unread = true;
                if inner
                    .focus
                    .as_ref()
                    .is_some_and(|focus| focus.item_id == item_id)
                {
                    inner.focus = None;
                }
            }
            Some(Request::Back) => {
                let mut inner = lock(&self.inner);
                // 栈里那条已经不在列表里了（撤回、被裁掉）就继续弹。
                while let Some(item_id) = inner.back.pop() {
                    if entries.iter().any(|entry| entry.id == item_id) {
                        inner.focus = Some(TimelineFocus {
                            item_id,
                            highlight: false,
                        });
                        break;
                    }
                }
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.bottom = inner.back.is_empty();
            }
        }
    }

    fn begin_unread(
        &self,
        entries: &Vector<TimelineEntry>,
        position: Option<String>,
        unread: u32,
        mention: bool,
    ) {
        let mut inner = lock(&self.inner);
        inner.unread_state = UnreadJumpState::Loading;
        let wanted = if mention {
            Wanted::NextMention { position, unread }
        } else {
            Wanted::FirstUnread { position, unread }
        };
        match resolve(entries, &wanted, inner.reached_start, &inner.seen_mentions) {
            Target::Found(item_id) => self.focus_unread(&mut inner, entries, item_id, mention),
            Target::Missing => {
                inner.state = JumpState::Loading;
                inner.wanted = Some(wanted);
            }
            Target::NotFound => {
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.unread_state = UnreadJumpState::Unavailable {
                    reason: UnreadJumpUnavailable::NotFound,
                };
            }
        }
    }

    fn focus_unread(
        &self,
        inner: &mut Inner,
        entries: &Vector<TimelineEntry>,
        item_id: String,
        mention: bool,
    ) {
        if let Some(last) = entries.last() {
            inner.back.push(last.id.clone());
        }
        if mention {
            inner.seen_mentions.insert(item_id.clone());
        } else {
            inner.dismiss_unread = true;
        }
        inner.focus = Some(TimelineFocus {
            item_id,
            highlight: true,
        });
        inner.state = JumpState::Idle;
        inner.wanted = None;
        inner.unread_state = UnreadJumpState::Idle;
    }

    /// 每批进壳之前对一次表：要跳的原消息出现在这一批里就定下来，栈里已经不在的条目去掉。
    pub(in crate::chat) fn progress(&self, entries: &Vector<TimelineEntry>) {
        let mut inner = lock(&self.inner);
        inner
            .back
            .retain(|item_id| entries.iter().any(|entry| &entry.id == item_id));
        if inner.state != JumpState::Loading {
            return;
        }
        let Some(wanted) = inner.wanted.clone() else {
            return;
        };
        match &wanted {
            Wanted::Event { event, .. } => {
                let Some(item_id) = find(entries, event) else {
                    return;
                };
                let Wanted::Event { from, .. } = wanted else {
                    return;
                };
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.back.push(from);
                inner.focus = Some(TimelineFocus {
                    item_id,
                    highlight: true,
                });
            }
            Wanted::FirstUnread { .. } | Wanted::NextMention { .. } => {
                let mention = matches!(wanted, Wanted::NextMention { .. });
                match resolve(entries, &wanted, inner.reached_start, &inner.seen_mentions) {
                    Target::Found(item_id) => {
                        self.focus_unread(&mut inner, entries, item_id, mention);
                    }
                    Target::Missing => {}
                    Target::NotFound => {
                        inner.state = JumpState::Idle;
                        inner.wanted = None;
                        inner.unread_state = UnreadJumpState::Unavailable {
                            reason: UnreadJumpUnavailable::NotFound,
                        };
                    }
                }
            }
        }
    }

    /// 转发循环被叫醒时做的一步：处理挂着的请求，还找不到就接着翻页。
    pub(in crate::chat) fn tick(
        self: &Arc<Self>,
        runtime: &Handle,
        room: &Arc<ChatRoom>,
        entries: &Vector<TimelineEntry>,
    ) {
        self.begin(entries);
        self.settle(entries);
        if self.state() == JumpState::Loading {
            if lock(&self.inner).reached_start {
                self.give_up();
            } else {
                self.paginate(runtime, room.clone());
            }
        }
    }

    /// 翻页出错或已经到开头：按当前条目收尾。
    fn settle(&self, entries: &Vector<TimelineEntry>) {
        let failed = lock(&self.inner).failed.take();
        if let Some(reason) = failed {
            self.give_up_unread(reason);
            return;
        }
        let mut inner = lock(&self.inner);
        if inner.state != JumpState::Loading || !inner.reached_start {
            return;
        }
        let Some(wanted) = inner.wanted.clone() else {
            return;
        };
        if !matches!(
            wanted,
            Wanted::FirstUnread { .. } | Wanted::NextMention { .. }
        ) {
            return;
        }
        // 到开头后的目标；@ 找不到时落回最早未读。
        match resolve(entries, &wanted, true, &inner.seen_mentions) {
            Target::Found(item_id) => {
                let mention = matches!(wanted, Wanted::NextMention { .. });
                self.focus_unread(&mut inner, entries, item_id, mention);
            }
            Target::Missing | Target::NotFound => {
                if matches!(wanted, Wanted::NextMention { .. }) {
                    let fallback = Wanted::FirstUnread {
                        position: None,
                        unread: 0,
                    };
                    if let Target::Found(item_id) =
                        resolve(entries, &fallback, true, &inner.seen_mentions)
                    {
                        self.focus_unread(&mut inner, entries, item_id, false);
                        return;
                    }
                }
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.unread_state = UnreadJumpState::Unavailable {
                    reason: UnreadJumpUnavailable::NotFound,
                };
            }
        }
    }

    /// 翻到开头（或翻页出错）还没有：这一版找不到。
    pub(super) fn give_up(&self) {
        self.give_up_unread(UnreadJumpUnavailable::NotFound);
    }

    pub(super) fn give_up_unread(&self, reason: UnreadJumpUnavailable) {
        let mut inner = lock(&self.inner);
        if inner.state == JumpState::Loading {
            inner.state = JumpState::Unavailable;
            inner.wanted = None;
            inner.unread_state = UnreadJumpState::Unavailable { reason };
        }
    }
}
