//! 底栏和面板配置的账号数据（ADR-0052）：全局 `app.wonton.layout`，房间 `app.wonton.room_panel`。
//!
//! 内容按原始 JSON 收着：改一项只动那一处，其余字段原样写回，别处（另一台设备上的新版本 App）存的
//! 取值不会被冲掉。类型和 `wonton-protocol` 的 `LayoutSettings`/`RoomPanel` 相同。

use futures_util::{Stream, StreamExt, stream};
use matrix_sdk::ruma::{
    RoomId,
    events::{
        False, GlobalAccountDataEvent, GlobalAccountDataEventContent, RoomAccountDataEvent,
        RoomAccountDataEventContent, StaticEventContent,
    },
    serde::JsonObject,
};
use serde::{Deserialize, Serialize};

use crate::{Error, MatrixSession, Result};

/// 存底栏和主面板的 Matrix 全局账号数据事件类型，内容和 `wonton-protocol` 的 `LayoutSettings` 相同。
pub const LAYOUT_EVENT: &str = "app.wonton.layout";

/// 存一个房间的面板的房间账号数据事件类型，内容和 `wonton-protocol` 的 `RoomPanel` 相同。
pub const ROOM_PANEL_EVENT: &str = "app.wonton.room_panel";

/// [`LAYOUT_EVENT`] 的内容，字段原样保留。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayoutContent(pub JsonObject);

impl StaticEventContent for LayoutContent {
    const TYPE: &'static str = LAYOUT_EVENT;
    type IsPrefix = False;
}

impl GlobalAccountDataEventContent for LayoutContent {
    fn event_type(&self) -> matrix_sdk::ruma::events::GlobalAccountDataEventType {
        LAYOUT_EVENT.into()
    }
}

/// [`ROOM_PANEL_EVENT`] 的内容，字段原样保留。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoomPanelContent(pub JsonObject);

impl StaticEventContent for RoomPanelContent {
    const TYPE: &'static str = ROOM_PANEL_EVENT;
    type IsPrefix = False;
}

impl RoomAccountDataEventContent for RoomPanelContent {
    fn event_type(&self) -> matrix_sdk::ruma::events::RoomAccountDataEventType {
        ROOM_PANEL_EVENT.into()
    }
}

impl MatrixSession {
    /// 本机存着的配置；还没同步到、或者没设过时是 `None`。
    pub async fn layout(&self) -> Result<Option<LayoutContent>> {
        self.client
            .account()
            .account_data::<LayoutContent>()
            .await
            .map_err(Error::from)?
            .map(|raw| {
                raw.deserialize()
                    .map_err(|e| Error::Upstream(e.to_string()))
            })
            .transpose()
    }

    /// 服务器上的内容，不管本机存没存过。
    pub async fn server_layout(&self) -> Result<Option<LayoutContent>> {
        self.client
            .account()
            .fetch_account_data_static::<LayoutContent>()
            .await
            .map_err(Error::from)?
            .map(|raw| {
                raw.deserialize()
                    .map_err(|e| Error::Upstream(e.to_string()))
            })
            .transpose()
    }

    /// 从服务器取最新的内容，按 `change` 改完写回，返回写回的内容。
    ///
    /// 取服务器上的而不是本机存的：另一台设备刚存的值也在里面，改一项不会把它冲掉。
    pub async fn update_layout(
        &self,
        change: impl FnOnce(&mut JsonObject),
    ) -> Result<LayoutContent> {
        let mut content = self.server_layout().await?.unwrap_or_default();
        change(&mut content.0);
        self.client
            .account()
            .set_account_data(content.clone())
            .await
            .map_err(Error::from)?;
        Ok(content)
    }

    /// 这条账号数据每次变化时给出新的完整内容，先给服务器上的这一份。
    pub async fn layout_changes(&self) -> Result<impl Stream<Item = LayoutContent> + use<>> {
        let handler = self
            .client
            .observe_events::<GlobalAccountDataEvent<LayoutContent>, ()>();
        let mut events = Box::pin(handler.subscribe());
        let changes = stream::poll_fn(move |context| {
            let _handler = &handler;
            events.as_mut().poll_next(context)
        });
        let initial = self.server_layout().await?.into_iter().collect::<Vec<_>>();
        Ok(stream::iter(initial).chain(changes.map(|event| event.0.content)))
    }

    /// 一个房间本机存着的面板；还没同步到、或者没设过时是 `None`。
    pub async fn room_panel(&self, room_id: &RoomId) -> Result<Option<RoomPanelContent>> {
        let room = self.joined(room_id)?;
        room.account_data_static::<RoomPanelContent>()
            .await
            .map_err(Error::from)?
            .map(|raw| {
                raw.deserialize()
                    .map(|event| event.content)
                    .map_err(|e| Error::Upstream(e.to_string()))
            })
            .transpose()
    }

    /// 从本机存的内容改起，写回房间账号数据，返回写回的内容。
    ///
    /// 房间账号数据没有取服务器最新一份的接口：按本机存着的改（同步会把它带到最新），写回整份。
    pub async fn update_room_panel(
        &self,
        room_id: &RoomId,
        change: impl FnOnce(&mut JsonObject),
    ) -> Result<RoomPanelContent> {
        let room = self.joined(room_id)?;
        let mut content = room
            .account_data_static::<RoomPanelContent>()
            .await
            .map_err(Error::from)?
            .map(|raw| {
                raw.deserialize()
                    .map(|event| event.content)
                    .map_err(|e| Error::Upstream(e.to_string()))
            })
            .transpose()?
            .unwrap_or_default();
        change(&mut content.0);
        room.set_account_data(content.clone())
            .await
            .map_err(Error::from)?;
        Ok(content)
    }

    /// 这个房间的面板每次变化时给出新的完整内容。
    pub fn room_panel_changes(
        &self,
        room_id: &RoomId,
    ) -> impl Stream<Item = RoomPanelContent> + use<> {
        let handler = self
            .client
            .observe_room_events::<RoomAccountDataEvent<RoomPanelContent>, ()>(room_id);
        let mut events = Box::pin(handler.subscribe());
        stream::poll_fn(move |context| {
            let _handler = &handler;
            events.as_mut().poll_next(context)
        })
        .map(|event| event.0.content)
    }
}
