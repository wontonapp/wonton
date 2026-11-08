//! 聊天显示方式的账号数据（ADR-0060）：存在全局账号数据 `app.wonton.chat_layout`，跟着账号走。
//!
//! 内容按原始 JSON 收着：改一项只动那一处，其余字段原样写回，别处（另一台设备上的新版本 App）存的取值不会被冲掉。

use futures_util::{Stream, StreamExt, stream};
use matrix_sdk::ruma::{
    events::{False, GlobalAccountDataEvent, GlobalAccountDataEventContent, StaticEventContent},
    serde::JsonObject,
};
use serde::{Deserialize, Serialize};

use crate::{Error, MatrixSession, Result};

/// 存用户选择的 Matrix 全局账号数据事件类型，内容和 `wonton-protocol` 的 `ChatLayoutSettings` 相同。
pub const CHAT_LAYOUT_EVENT: &str = "app.wonton.chat_layout";

/// [`CHAT_LAYOUT_EVENT`] 的内容，字段原样保留。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChatLayoutContent(pub JsonObject);

impl StaticEventContent for ChatLayoutContent {
    const TYPE: &'static str = CHAT_LAYOUT_EVENT;
    type IsPrefix = False;
}

impl GlobalAccountDataEventContent for ChatLayoutContent {
    fn event_type(&self) -> matrix_sdk::ruma::events::GlobalAccountDataEventType {
        CHAT_LAYOUT_EVENT.into()
    }
}

impl MatrixSession {
    /// 本机存着的选择；还没同步到、或者没设过时是 `None`。
    pub async fn chat_layout(&self) -> Result<Option<ChatLayoutContent>> {
        self.client
            .account()
            .account_data::<ChatLayoutContent>()
            .await
            .map_err(Error::from)?
            .map(|raw| {
                raw.deserialize()
                    .map_err(|e| Error::Upstream(e.to_string()))
            })
            .transpose()
    }

    /// 服务器上的内容，不管本机存没存过。
    async fn fetched_chat_layout(&self) -> Result<Option<ChatLayoutContent>> {
        self.client
            .account()
            .fetch_account_data_static::<ChatLayoutContent>()
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
    pub async fn update_chat_layout(
        &self,
        change: impl FnOnce(&mut JsonObject),
    ) -> Result<ChatLayoutContent> {
        let mut content = self.fetched_chat_layout().await?.unwrap_or_default();
        change(&mut content.0);
        self.client
            .account()
            .set_account_data(content.clone())
            .await
            .map_err(Error::from)?;
        Ok(content)
    }

    /// 这条账号数据每次变化时给出新的完整内容，先给服务器上的这一份。
    ///
    /// 初值从服务器取：换设备、或者本机还没同步到时，第一次就有值。先订阅再取：两步之间到的变化不会漏掉，
    /// 订阅句柄由返回的流一直带着。
    pub async fn chat_layout_changes(
        &self,
    ) -> Result<impl Stream<Item = ChatLayoutContent> + use<>> {
        let handler = self
            .client
            .observe_events::<GlobalAccountDataEvent<ChatLayoutContent>, ()>();
        let mut events = Box::pin(handler.subscribe());
        let changes = stream::poll_fn(move |context| {
            let _handler = &handler;
            events.as_mut().poll_next(context)
        });
        let initial = self
            .fetched_chat_layout()
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        Ok(stream::iter(initial).chain(changes.map(|event| event.0.content)))
    }
}
