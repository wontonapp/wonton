//! 我的应用配置的账号数据（ADR-0095）：全局 `app.wonton.layout`。
//!
//! 内容按原始 JSON 收着：改一项只动那一处，其余字段原样写回，别处（另一台设备上的新版本 App）存的
//! 取值不会被冲掉。类型和 `wonton-protocol` 的 `LayoutSettings` 相同。

use futures_util::{Stream, StreamExt, stream};
use matrix_sdk::ruma::{
    events::{False, GlobalAccountDataEvent, GlobalAccountDataEventContent, StaticEventContent},
    serde::JsonObject,
};
use serde::{Deserialize, Serialize};

use crate::{Error, MatrixSession, Result};

/// 存我的应用的 Matrix 全局账号数据事件类型，内容和 `wonton-protocol` 的 `LayoutSettings` 相同。
pub const LAYOUT_EVENT: &str = "app.wonton.layout";

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
}
