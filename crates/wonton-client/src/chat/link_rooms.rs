//! 单个房间的订阅：开始一条时间线、打开时间线、定位会话与发送目标、推送状态和清屏入口。

use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_matrix::{ChatRoom, Error, MatrixSession, OwnedRoomId, OwnedUserId};
use wonton_protocol::{CoreError, PushStatus, TimelineTarget};

use super::core_error;
use super::menu;
use super::recovery;
use super::target;
#[cfg(feature = "test-hooks")]
use super::timeline;
use super::util::{invalid, lock};
use super::{ChatLink, TimelineKey, TimelineSource, TimelineSubscription, open_chat_room};
use crate::l10n::{CATALOG, language};

impl ChatLink {
    pub(super) fn confirm_recovery_key_saved(&self) -> Result<(), CoreError> {
        self.storage.forget_recovery_key()?;
        self.view.send_modify(|view| {
            view.encryption.recovery_key = None;
            recovery::refresh(&mut view.encryption);
        });
        Ok(())
    }

    pub(crate) fn begin_timeline(
        self: &Arc<Self>,
        target: &TimelineTarget,
    ) -> Result<target::TimelineOpening, CoreError> {
        let key = TimelineKey::parse(target)?;
        let mut open = lock(&self.open);
        let id = open.begin(&key);
        Ok(target::TimelineOpening {
            key,
            id,
            chat: Arc::downgrade(self),
        })
    }

    /// 每份订阅钉住所需的房间；取消或释放不影响其他订阅。
    ///
    /// `room_view` 是 tabs 合成的房间视图（顶栏的 widget 入口），每批时间线增量
    /// 都带上当前值；变了也发一批（增量为空）。
    pub(crate) async fn open_timeline(
        self: &Arc<Self>,
        opening: target::TimelineOpening,
        room_view: watch::Receiver<Option<Arc<crate::pages::RoomView>>>,
    ) -> Result<Arc<TimelineSubscription>, CoreError> {
        let key = opening.key.clone();
        let _opening = self.opening.lock().await;
        if !lock(&self.open).contains_request(opening.id) {
            return Err(target::not_subscribed(&key));
        }
        let session = self.current()?;
        let room_id = key.room_id.clone();
        if *self.bots.foreground.borrow() {
            self.bots.refresh_all();
        }
        let existing = lock(&self.open)
            .get(&key)
            .and_then(|open| open.room.upgrade());
        let room = match existing {
            Some(room) => room,
            None => Arc::new(
                open_chat_room(&session, &key)
                    .await
                    .map_err(|error| core_error(&error))?,
            ),
        };
        let menus = Arc::new(Mutex::new(menu::Menus::default()));
        let mut open = lock(&self.open);
        if !open.contains_request(opening.id) {
            return Err(target::not_subscribed(&key));
        }
        // 进房间时重新核对一次支持声明：Bot 可能是刚连上才写的（ADR-0093 §1）。
        let support = self.side_chat_support.clone();
        support.refresh(&self.runtime, &session, &room_id);
        // 链接的类别要认插件页，插件列表还一次没取过时先取一次（取过或正在取就不重复，免得打乱别处的节奏）。
        let plugins = self.plugins.view();
        if plugins.installed.is_none() && !plugins.refreshing {
            self.plugins.refresh_installed();
        }
        let id = opening.id;
        let subscription = TimelineSubscription::start(
            &self.runtime,
            TimelineSource {
                room: room.clone(),
                room_id: room_id.clone(),
                confirmations: self.room_cards.subscribe(),
                plugins: self.plugins.clone(),
                push_previews: self.push_previews.clone(),
                thread_root: key.thread_root.clone(),
                previews: self.previews.clone(),
                menus: menus.clone(),
                room_view,
                chat: self.view.subscribe(),
                support,
                bots: self.bots.clone(),
                session,
            },
            opening,
        );
        open.insert(target::OpenTimeline {
            id,
            key,
            room: Arc::downgrade(&room),
            menus: Arc::downgrade(&menus),
            subscription: Arc::downgrade(&subscription),
        });
        Ok(subscription)
    }

    pub(super) fn start_direct_chat(self: &Arc<Self>, user_id: &str) -> Result<(), CoreError> {
        let user: OwnedUserId = user_id
            .parse()
            .map_err(|_| invalid(&l10n!(CATALOG, language(), "chat-invalid-user-id")))?;
        let session = self.current()?;
        if session.direct_chat_with(&user).is_some() {
            return Ok(());
        }
        // 房间不加密：推送只带房间和事件 ID，内容由手机在房间里直接读。
        self.spawn_op("chat-op-direct-chat-failed", async move {
            session.create_direct_chat(&user, false).await.map(drop)
        });
        Ok(())
    }

    /// 推送的状态由 `push` 模块维护，放在聊天的 ViewModel 里。
    pub(crate) fn set_push(&self, status: PushStatus) {
        self.view.send_modify(|view| view.push = status);
    }

    pub(crate) fn current(&self) -> Result<Arc<MatrixSession>, CoreError> {
        lock(&self.session)
            .clone()
            .ok_or_else(|| CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "chat-not-signed-in"),
            })
    }

    pub(super) fn target(
        &self,
        room_id: &str,
    ) -> Result<(Arc<MatrixSession>, OwnedRoomId), CoreError> {
        let room_id = room_id.parse::<OwnedRoomId>().map_err(|e| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "chat-invalid-room-id",
                error = e.to_string()
            ))
        })?;
        Ok((self.current()?, room_id))
    }

    /// 要往房间里发东西时用它，只读的房间拒绝。
    pub(super) fn send_target(
        &self,
        room_id: &str,
    ) -> Result<(Arc<MatrixSession>, OwnedRoomId), CoreError> {
        let read_only = self
            .view
            .borrow()
            .rooms
            .iter()
            .any(|row| row.room_id == room_id && row.is_read_only);
        if read_only {
            return Err(invalid(&l10n!(CATALOG, language(), "chat-read-only-room")));
        }
        self.target(room_id)
    }

    pub(super) fn opened(&self, key: &TimelineKey) -> Result<Arc<ChatRoom>, CoreError> {
        lock(&self.open)
            .get(key)
            .and_then(|open| open.room.upgrade())
            .ok_or_else(|| target::not_subscribed(key))
    }

    /// 测试用：像 SDK 丢掉活事件那样清掉一条已打开时间线的条目，事件缓存不动，等补检重建。清空对壳是无声的
    /// （不发 `Clear`，也不走 SDK 清空的重建），只有补检发现后才让壳清屏。只在 `test-hooks` 构建里有调用方。
    #[cfg(feature = "test-hooks")]
    pub(crate) async fn test_clear_timeline(
        &self,
        target: &TimelineTarget,
    ) -> Result<(), CoreError> {
        let key = TimelineKey::parse(target)?;
        let room = self.opened(&key)?;
        timeline::mark_silent_clear(key.room_id.clone(), key.thread_root.clone());
        room.timeline().await.clear().await;
        Ok(())
    }

    /// 已打开的目标共用时间线；临时操作独立持有房间监听。
    pub(super) async fn room(&self, key: &TimelineKey) -> Result<Arc<ChatRoom>, Error> {
        if let Some(room) = lock(&self.open)
            .get(key)
            .and_then(|open| open.room.upgrade())
        {
            return Ok(room);
        }
        let session = self.current().map_err(|e| Error::NotReady(e.to_string()))?;
        Ok(Arc::new(open_chat_room(&session, key).await?))
    }
}
