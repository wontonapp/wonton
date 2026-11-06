//! 全局资料和同步中的资料变化；不要求打开某个房间的时间线。

use futures_util::{Stream, stream};
use matrix_sdk::ruma::{
    OwnedUserId, RoomId, UserId,
    events::room::member::{MembershipState, OriginalSyncRoomMemberEvent},
};
use serde_json::{Map, Value};
use tokio::sync::{broadcast::error::RecvError, mpsc};

use crate::{Error, MatrixSession, Result};

/// Matrix 全局资料；扩展字段原样交给调用方解释。
#[derive(Clone, Debug, PartialEq)]
pub struct UserProfile {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub fields: Map<String, Value>,
}

impl MatrixSession {
    /// 读取完整资料，包括服务器返回的所有自定义字段。
    pub async fn fetch_user_profile(&self, user_id: &UserId) -> Result<UserProfile> {
        let response = self.client.account().fetch_user_profile_of(user_id).await?;
        let mut fields: Map<String, Value> = response.into_iter().collect();
        let display_name = profile_string(fields.remove("displayname"))?;
        let avatar_url = profile_string(fields.remove("avatar_url"))?;
        Ok(UserProfile {
            display_name,
            avatar_url,
            fields,
        })
    }

    /// 观察已同步房间的资料变化；不改变选中的房间或它的订阅。
    pub fn profile_changes(&self) -> impl Stream<Item = OwnedUserId> + Send + use<> {
        let (sender, receiver) = mpsc::unbounded_channel();
        // SDK 的 StateOriginal 分派同时覆盖房间列表的 state 和时间线的成员事件。
        let handle = self
            .client
            .add_event_handler(move |event: OriginalSyncRoomMemberEvent| {
                let sender = sender.clone();
                async move {
                    if member_profile_changed(&event) {
                        let _ = sender.send(event.state_key);
                    }
                }
            });
        let guard = self.client.event_handler_drop_guard(handle);
        stream::unfold((receiver, guard), |(mut receiver, guard)| async move {
            receiver.recv().await.map(|user| (user, (receiver, guard)))
        })
    }

    /// 观察房间里正在输入的用户；释放流时同步释放 SDK 事件处理器。
    pub fn typing_notifications(
        &self,
        room_id: &RoomId,
    ) -> Result<impl Stream<Item = Vec<OwnedUserId>> + Send + use<>> {
        let room = self.client.get_room(room_id).ok_or_else(|| {
            Error::NotReady("这个聊天还没有同步到这台设备，稍等一会儿再试".to_owned())
        })?;
        let (guard, receiver) = room.subscribe_to_typing_notifications();
        Ok(stream::unfold(
            (receiver, guard),
            |(mut receiver, guard)| async move {
                loop {
                    match receiver.recv().await {
                        Ok(users) => return Some((users, (receiver, guard))),
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => return None,
                    }
                }
            },
        ))
    }
}

fn profile_string(value: Option<Value>) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(Error::Upstream("Matrix 资料字段不是字符串".to_owned())),
    }
}

pub(crate) fn member_profile_changed(event: &OriginalSyncRoomMemberEvent) -> bool {
    event.content.membership == MembershipState::Join
        && event
            .unsigned
            .prev_content
            .as_ref()
            .is_some_and(|previous| {
                previous.membership == MembershipState::Join
                    && (previous.displayname != event.content.displayname
                        || previous.avatar_url != event.content.avatar_url)
            })
}
