//! 一个已打开房间的时间线：收发文字和图片（发送失败的可以重发、取消）、按增量观察条目、向前翻页、发已读回执。

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use matrix_sdk::{
    Room,
    ruma::{
        OwnedEventId, RoomId, UserId,
        api::client::state::get_state_events,
        events::{reaction::ReactionEventContent, relation::Annotation},
    },
    send_queue::SendHandle,
};
use matrix_sdk_ui::timeline::{EventTimelineItem, Timeline};
use tokio::{
    sync::{Notify, watch},
    task::JoinHandle,
};
use wonton_protocol::{ChoiceSend, ChoicesState};

use crate::{
    Error, Result,
    message::{MemberNames, message},
};

/// 发图前向服务器要上传上限最多等多久。
const UPLOAD_LIMIT_WAIT: Duration = Duration::from_secs(5);

/// 要发送的图片。
#[derive(Clone, Debug)]
pub struct ImageUpload {
    /// 文件名，接收方保存时使用。
    pub filename: String,
    /// MIME 类型，必须是 `image/*`。
    pub mime_type: String,
    /// 原图字节。
    pub data: Vec<u8>,
    /// 像素宽度，接收方据此预留版面；未知时为 `None`。
    pub width: Option<u32>,
    /// 像素高度。
    pub height: Option<u32>,
    /// 发送端生成的缩略图。加密房间里服务器做不了缩略图，对方预览只能下载原图，所以由发送端给。
    pub thumbnail: Option<ImageThumbnail>,
}

/// 发送端生成的缩略图。
#[derive(Clone, Debug)]
pub struct ImageThumbnail {
    /// MIME 类型，如 `image/jpeg`。
    pub mime_type: String,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// 缓存里有、时间线里一直没有的事件。
#[derive(Clone, Debug)]
pub struct MissedEvent {
    /// 事件 ID。
    pub id: OwnedEventId,
    /// 从第一次看到它缺席到现在过了多久（本机单调时钟）。
    pub absent: Duration,
}

/// @ 列表里的一个在场成员。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionMember {
    pub user_id: String,
    pub display_name: String,
    /// 头像的 `mxc://`。
    pub avatar: Option<String>,
}

/// 一个已打开的房间。
pub struct ChatRoom {
    pub(crate) room: Room,
    /// 时间线。事件缓存被清空重来时要换一条，房间对象本身不换（发消息、翻页都还走它）。
    timeline: tokio::sync::RwLock<Arc<Timeline>>,
    /// 缓存里有、时间线里一直没有的事件第一次被看到缺席的时间。
    missing: tokio::sync::Mutex<HashMap<OwnedEventId, Instant>>,
    names: watch::Receiver<Arc<MemberNames>>,
    /// 房间有更新时在后台刷新显示名；房间关掉时停止。
    refresh: JoinHandle<()>,
    /// 在后台向服务器取全部成员，取到为止，补上时间线里发送者的资料；房间关掉时停止。
    profiles: JoinHandle<()>,
    /// 这条时间线看的是哪一段：整个房间平铺、主对话（话题收起）或一个话题（ADR-0093）。
    pub(crate) focus: Focus,
    pub(crate) watch: Option<crate::rooms::RoomWatch>,
}

impl Drop for ChatRoom {
    fn drop(&mut self) {
        self.refresh.abort();
        self.profiles.abort();
    }
}

impl ChatRoom {
    /// 打开房间只读本机已有的数据，不等网络：成员没同步全时 SDK 的 `members` 会去服务器拉（超时 60 秒、
    /// 重试 3 次），隧道断开时打开房间要等很久，本机缓存的历史也出不来。
    ///
    /// 显示名先用本机已有的成员事件（同步时随消息发送者一起来）。打开后在后台向服务器取全部成员，取到后时间线
    /// 里发送者的资料随之补上（条目以 `Set` 增量更新）：新登录的设备上，房间列表同步只带最新一条的发送者，更早
    /// 的发送者要靠这一步才有显示名。取不到（比如隧道断开）不影响聊天，发送者先显示账号名，隔一会儿再取。
    ///
    /// 打开前只经房间列表同步过最新一条时，更早的消息隔着缺口，服务器不一定随房间订阅补发，要向前翻页
    /// （[`Self::paginate_backwards`]，界面滚到顶时自动翻）。
    pub(crate) async fn open(room: Room) -> Result<Self> {
        Self::open_with(room, Focus::Flat).await
    }

    /// 同 [`Self::open`]，时间线看 `focus` 那一段。
    pub(crate) async fn open_with(room: Room, focus: Focus) -> Result<Self> {
        let timeline = Arc::new(build_timeline(&room, &focus).await?);
        let (names_tx, names) = watch::channel(Arc::new(local_member_names(&room).await));
        let fetched = Arc::new(Notify::new());
        let refresh = tokio::spawn(refresh_member_names(
            room.clone(),
            names_tx,
            fetched.clone(),
        ));
        let profiles = tokio::spawn(fetch_members_until_done(
            room.clone(),
            timeline.clone(),
            fetched,
        ));
        Ok(Self {
            room,
            timeline: tokio::sync::RwLock::new(timeline),
            missing: tokio::sync::Mutex::new(HashMap::new()),
            names,
            refresh,
            profiles,
            focus,
            watch: None,
        })
    }

    /// 换一条时间线：事件缓存被清空重来以后（同步从头开始、服务器丢了位置），旧时间线收不到清空期间的事件，
    /// 新的一条从缓存重新读一遍。房间对象不换，发消息和翻页都还走它。
    pub async fn rebuild_timeline(&self) -> Result<()> {
        // 整段持写锁：换的工夫别人要时间线（翻页、比对缓存）就等新的，不拿到马上要被换掉的那一条。
        let mut slot = self.timeline.write().await;
        let timeline = Arc::new(build_timeline(&self.room, &self.focus).await?);
        tokio::spawn(fetch_members_until_done(
            self.room.clone(),
            timeline.clone(),
            Arc::new(Notify::new()),
        ));
        *slot = timeline;
        Ok(())
    }

    /// 现在这一条时间线。
    pub async fn timeline(&self) -> Arc<Timeline> {
        self.timeline.read().await.clone()
    }

    /// 房间 ID。
    pub fn room_id(&self) -> &RoomId {
        self.room.room_id()
    }

    /// 发送文字，`mentions` 是输入框里选中的人（写进 `m.mentions` 和正文的胶囊，见 [`Self::mention_content`]）。
    /// 消息先以本地回显出现在时间线里，发送结果见 [`Message::status`]。
    pub async fn send_text(&self, body: &str, mentions: &[String]) -> Result<SendHandle> {
        ensure_recipient_devices(&self.room).await?;
        let names = self.names();
        let users = crate::mentions::selected(mentions, &names);
        let content = self.mention_content(body, &users);
        Ok(self.timeline().await.send(content.into()).await?)
    }

    /// 发送文字，事件内容里带上额外字段（链接预览的 `com.beeper.linkpreviews`，ADR-0065）。其余同
    /// [`Self::send_text`]。
    pub async fn send_text_with_extra(
        &self,
        body: &str,
        mentions: &[String],
        extra: serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        ensure_recipient_devices(&self.room).await?;
        let names = self.names();
        let users = crate::mentions::selected(mentions, &names);
        let content = self.mention_content(body, &users);
        self.timeline()
            .await
            .send_with_extra_content(content.into(), Some(extra))
            .await?;
        Ok(())
    }

    /// 发一个房间状态事件；`state_key` 为空串时是房间级状态。内容原样发，不需要 ruma 认识事件类型。
    pub async fn invite(&self, user: &UserId) -> Result<()> {
        self.room.invite_user_by_id(user).await?;
        Ok(())
    }

    pub async fn send_state_event_raw(
        &self,
        event_type: &str,
        state_key: &str,
        content: serde_json::Value,
    ) -> Result<()> {
        self.room
            .send_state_event_raw(event_type, state_key, content)
            .await?;
        Ok(())
    }

    /// 向服务器取一个房间状态事件的原始 JSON（含 `sender`）；没有这个状态时返回 `None`。
    ///
    /// 自定义事件类型不在 Sliding Sync 的默认 `required_state` 里，本机缓存也不会有，所以直接问服务器。
    pub async fn fetch_state_event(
        &self,
        event_type: &str,
        state_key: &str,
    ) -> Result<Option<serde_json::Value>> {
        let response = self
            .room
            .client()
            .send(get_state_events::v3::Request::new(
                self.room_id().to_owned(),
            ))
            .await?;
        for raw in response.room_state {
            let value =
                serde_json::to_value(&raw).map_err(|error| Error::Upstream(error.to_string()))?;
            if value["type"] == event_type && value["state_key"] == state_key {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// 向服务器取一个房间状态事件的内容；没有这个状态时返回 `None`。
    pub async fn fetch_state_event_content(
        &self,
        event_type: &str,
        state_key: &str,
    ) -> Result<Option<serde_json::Value>> {
        Ok(self
            .fetch_state_event(event_type, state_key)
            .await?
            .map(|event| event["content"].clone()))
    }

    /// 撤回一条消息：理由随撤回事件发给服务器，房间里的人都能看到；撤回不能撤销。
    pub async fn redact(&self, event_id: &str, reason: &str) -> Result<()> {
        let event_id: OwnedEventId = event_id
            .parse()
            .map_err(|_| Error::InvalidInput(format!("不是事件 ID：{event_id}")))?;
        self.room.redact(&event_id, Some(reason), None).await?;
        Ok(())
    }

    /// 回答一条消息的选项（ADR-0109 第 8 节）：以本账号加选项那个表情，不往聊天里发消息。发送者自己先放的
    /// 表情就是选项；私聊或提到了本账号的才能答，答过一条后其余不能再答。还在发或发失败的先撤掉再发。
    ///
    /// 也回答 MSC4564 的状态消息（ADR-0082）：它的选项在状态对象里，点下去照样加那个表情，已选也照表情回填。
    /// 条目不在时间线里、选项没有了、不是问我的、已经答过、状态行已经不在等待时报 `InvalidInput`。
    pub async fn answer_agent_choice(&self, item_id: &str, key: &str) -> Result<()> {
        let timeline = self.timeline().await;
        let items = timeline.items().await;
        let item = items
            .iter()
            .find(|item| item.unique_id().0 == item_id)
            .ok_or_else(|| Error::NotInTimeline("这条消息不在时间线里".to_owned()))?;
        let event = item
            .as_event()
            .ok_or_else(|| Error::InvalidInput("这条不是消息".to_owned()))?;
        let names = self.names.borrow().clone();
        let message =
            message(event, &names).ok_or_else(|| Error::InvalidInput("这条不是消息".to_owned()))?;
        let status_choice = message
            .agent_run
            .as_ref()
            .is_some_and(|status| status.choices.iter().any(|choice| choice.key == key));
        let retry = if status_choice {
            let status = message
                .agent_run
                .as_ref()
                .expect("status_choice 说明有状态");
            // 状态行只在“等待”时有效：跑完了、失败了再点就是过期的回答。
            if status.state != wonton_protocol::AgentRunState::Waiting {
                return Err(Error::InvalidInput("这条状态行已经不在等待".to_owned()));
            }
            if status.choices.iter().any(|choice| choice.chosen) {
                return Err(Error::InvalidInput("这条已经答过了".to_owned()));
            }
            if !message.is_direct && !message.mentions_me {
                return Err(Error::InvalidInput("这条不是问我的".to_owned()));
            }
            status
                .choices
                .iter()
                .any(|choice| choice.key == key && matches!(choice.send, ChoiceSend::Failed { .. }))
        } else {
            let choices = message
                .choices()
                .ok_or_else(|| Error::InvalidInput("这条没有选项".to_owned()))?;
            let option = choices
                .options
                .iter()
                .find(|option| option.key == key)
                .ok_or_else(|| Error::InvalidInput("这个选项没有了".to_owned()))?;
            match choices.state {
                ChoicesState::Answered => {
                    return Err(Error::InvalidInput("这条已经答过了".to_owned()));
                }
                ChoicesState::Open => {}
                ChoicesState::NotForMe => {
                    return Err(Error::InvalidInput("这条不是问我的".to_owned()));
                }
                ChoicesState::Other => {}
            }
            matches!(option.send, ChoiceSend::Failed { .. })
        };
        let event_id = event
            .event_id()
            .map(ToOwned::to_owned)
            .ok_or_else(|| Error::InvalidInput("这条还没发出去".to_owned()))?;
        ensure_recipient_devices(&self.room).await?;
        if retry {
            // 不能只靠 toggle：失败的回显还在时会走“撤掉”那一支，正好把要重发的删掉。
            timeline.toggle_reaction(&event.identifier(), key).await?;
        }
        timeline
            .send(ReactionEventContent::new(Annotation::new(event_id, key.to_owned())).into())
            .await?;
        Ok(())
    }

    /// 设置本账号在房间里的输入状态。
    pub async fn typing_notice(&self, typing: bool) -> Result<()> {
        self.room.typing_notice(typing).await?;
        Ok(())
    }

    /// 不经发送队列发一条内容任意的 `m.room.message`，返回事件 ID。测试里扮演核心、发确认卡片和编辑用。
    #[cfg(feature = "testkit")]
    pub async fn send_raw_message(
        &self,
        content: serde_json::Value,
    ) -> Result<matrix_sdk::ruma::OwnedEventId> {
        self.send_raw_event("m.room.message", content).await
    }

    /// 同上，事件类型也自己给（表情回应是 `m.reaction`）。
    #[cfg(feature = "testkit")]
    pub async fn send_raw_event(
        &self,
        event_type: &str,
        content: serde_json::Value,
    ) -> Result<matrix_sdk::ruma::OwnedEventId> {
        let sent = self
            .room
            .send_raw(event_type, content)
            .await
            .map_err(|error| Error::Upstream(format!("发送失败：{error}")))?;
        Ok(sent.response.event_id)
    }

    /// 马上重发一条发送失败的消息（`item_id` 是 [`TimelineEntry::id`]）：服务器拒收的那条重新排回队首再发，连接
    /// 出错的那条不等自动重发。
    pub async fn retry_send(&self, item_id: &str) -> Result<()> {
        self.unsent(item_id)
            .await?
            .unwedge()
            .await
            .map_err(|error| Error::Upstream(format!("重发失败：{error}")))?;
        // `unwedge` 只唤醒队列，出错后停下的队列要重新打开。
        self.room.send_queue().set_enabled(true);
        Ok(())
    }

    /// 取消一条还没发出去的消息（排队中或发送失败的），这个房间排在它后面的接着发。
    pub async fn cancel_send(&self, item_id: &str) -> Result<()> {
        let aborted = self
            .unsent(item_id)
            .await?
            .abort()
            .await
            .map_err(|error| Error::Upstream(format!("取消发送失败：{error}")))?;
        if !aborted {
            return Err(Error::InvalidInput("这条已经发出去了".to_owned()));
        }
        // 取消的是连接出错的那条时，后面的不用等自动重发的间隔。
        self.room.send_queue().set_enabled(true);
        Ok(())
    }

    /// 还没发出去的一条在发送队列里的句柄。
    async fn unsent(&self, item_id: &str) -> Result<SendHandle> {
        let items = self.timeline().await.items().await;
        let item = items
            .iter()
            .find(|item| item.unique_id().0 == item_id)
            .ok_or_else(|| Error::InvalidInput(format!("时间线里没有这一条：{item_id}")))?;
        item.as_event()
            .and_then(EventTimelineItem::local_echo_send_handle)
            .ok_or_else(|| Error::InvalidInput("这条已经发出去了".to_owned()))
    }
}

mod actions;
#[cfg(test)]
mod bootstrap_tests;
mod focus;
mod members;
mod names;
mod pagination_status;
mod raw;
mod send;
#[cfg(test)]
mod tests;
mod timeline;
mod upload;
mod visible;

pub(crate) use focus::{Focus, build_timeline};
pub(crate) use members::{ensure_recipient_devices, local_member_names};
use members::{fetch_members_until_done, refresh_member_names};
pub use timeline::{BackPaginationStatus, MISSED_GRACE};
pub(crate) use upload::check_upload_size;
