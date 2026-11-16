//! widget 页的桥（ADR-0096）：页面发来的 widget API 报文交给驱动，驱动发给页面的包成合成 `MessageEvent` 的脚本，
//! 由壳订阅后原样注入同一个页面。壳不解析报文。

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_matrix::WidgetDriverHandle;
use wonton_protocol::{CoreError, PlacedTab, TabRef};

use super::super::{PagesLink, placed_tab_ref};
use super::reply_script;
use crate::l10n::{CATALOG, language};

/// widget API 的报文：有 `api`、`action`、`requestId`、`widgetId` 字段（MSC2762），不是 JSON-RPC。
pub(super) fn is_widget_message(value: &Value) -> bool {
    value.get("jsonrpc").is_none()
        && matches!(value["api"].as_str(), Some("fromWidget" | "toWidget"))
        && value["action"].is_string()
        && value["requestId"].is_string()
        && value["widgetId"].is_string()
}

pub(crate) fn subscribe(
    tabs: &PagesLink,
    page: &PlacedTab,
) -> Result<Arc<TabWidgetSubscription>, CoreError> {
    let tab = placed_tab_ref(page);
    if !matches!(
        page,
        PlacedTab::Page {
            tab: TabRef::Widget { .. }
        }
    ) {
        return Err(CoreError::InvalidInput {
            reason: l10n!(CATALOG, language(), "tabs-widget-not-declared"),
        });
    }
    let handle = tabs.widget_handle(tab).ok_or_else(|| CoreError::NotReady {
        reason: l10n!(CATALOG, language(), "tabs-bridge-page-not-open"),
    })?;
    Ok(Arc::new(TabWidgetSubscription {
        handle,
        cancelled: watch::Sender::new(false),
    }))
}

/// widget 驱动发给页面的消息（toWidget 请求、对 fromWidget 请求的应答），每一项是一段要注入页面的脚本。
#[derive(uniffi::Object)]
pub struct TabWidgetSubscription {
    handle: Arc<WidgetDriverHandle>,
    cancelled: watch::Sender<bool>,
}

#[uniffi::export]
impl TabWidgetSubscription {
    /// 等下一段脚本，壳原样注入同一个页面的主框架。widget 会话结束（页面关了、换了房间、退出登录）、
    /// 订阅取消或核心释放后返回 `None`。只能有一个人在等。
    pub async fn next(&self) -> Option<String> {
        let mut cancelled = self.cancelled.subscribe();
        loop {
            let message = tokio::select! {
                biased;
                _ = cancelled.wait_for(|cancelled| *cancelled) => return None,
                message = self.handle.recv() => message?,
            };
            // 驱动发的都是它自己序列化的 JSON；万一不是，这一条页面也认不出，跳过、留下日志。
            match serde_json::from_str::<Value>(&message) {
                Ok(message) => return Some(reply_script(message)),
                Err(error) => {
                    tracing::warn!("widget driver sent something that isn't JSON: {error}")
                }
            }
        }
    }

    /// 取消订阅，语义同 `HomeSubscription::cancel`（Swift 的 Task 取消不会传到 Rust）。
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
    }
}
