//! 把新卡片通知给收件人的设备（ADR-0016 的“推送”）：核心直接发 Web Push，不进任何 Matrix 房间。
//! 之后的状态变化只改确认表，设备从“等你确认”列表的长轮询里知道，这里只管开卡的那一条。

use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use wonton_protocol::{ConfirmationCard, DeviceConfirmationPush};

use super::confirm::Notifier;
use crate::push::Push;

/// 开卡时给收件人（主人或 Bot 的归属人）名下的每台已配对设备发确认推送。
pub struct DevicePush {
    push: Arc<Push>,
}

impl DevicePush {
    pub fn new(push: Arc<Push>) -> Self {
        Self { push }
    }
}

impl Notifier for DevicePush {
    fn post<'a>(
        &'a self,
        recipient: Option<&'a str>,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            // 名下没有任何已配对设备：没人能答，如实告诉调用方。
            if !self.push.has_devices(recipient) {
                return Err(match recipient {
                    Some(person) => format!("{person} 名下没有已配对的设备，没人能回答确认"),
                    None => "主人没有已配对的设备，没人能回答确认".to_owned(),
                });
            }
            let confirmation = DeviceConfirmationPush {
                id: card.id.clone(),
                title: card.title.clone(),
                expires_at_ms: card.expires_at_ms,
                origin: card.origin.clone(),
            };
            // 推送没发出去不拦卡片：它已在确认表里，App 打开时从列表照样看得到。
            if let Err(error) = self.push.send_confirmation(recipient, &confirmation).await {
                tracing::warn!(
                    confirmation = %card.id,
                    person = recipient.unwrap_or("owner"),
                    %error,
                    "确认推送没发出去，App 靠列表取卡片"
                );
            }
            Ok(())
        }
        .boxed()
    }
}
