//! 推送代理（ADR-0003 的“设计：加密推送”）：已配对设备经隧道登记推送订阅；Tuwunel 有新消息时调推送代理，
//! 核心把通知加密给这台设备，按 Web Push 发到它的推送地址。
//!
//! 核心只经标准接口和别的组件说话（ADR-0015）：Tuwunel 调的是 Matrix Push Gateway API，推送服务收的是
//! Web Push（RFC 8030、RFC 8291）。推送里没有正文：聊天是端到端加密的，App 经隧道取回消息后在本机解密。

mod notification;
mod store;
mod webpush;

use std::path::PathBuf;
use std::sync::Arc;

use iroh::EndpointId;
use serde_json::json;
use wonton_protocol::DeviceConfirmationPush;

pub use notification::{DevicePayload, Notification, NotifyRequest, NotifyResponse};
pub use store::{Registration, Subscription};

use crate::device::{Device, Devices};
use crate::plugin::egress::core_proxy::CoreProxy;
use store::Registrations;
use webpush::{Delivery, WebPush};

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("{0}")]
    Invalid(String),
    #[error("读写推送登记表 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("推送登记表 {path} 已损坏：{message}")]
    Corrupt { path: PathBuf, message: String },
    #[error("推送服务暂时不可用：{0}")]
    Unavailable(String),
    #[error("准备发推送的 HTTP 客户端失败：{0}")]
    Client(String),
}

pub struct Push {
    /// 这台服务器的 endpoint ID，写进每条推送。
    server: String,
    registrations: Registrations,
    devices: Arc<Devices>,
    sender: WebPush,
}

impl Push {
    /// 读取登记表（`core/push.json`）。`devices` 是隧道用的同一份设备登记表：已吊销的设备收不到推送。`server` 是
    /// 隧道的 endpoint ID。
    pub fn open(
        path: impl Into<PathBuf>,
        devices: Arc<Devices>,
        server: EndpointId,
    ) -> Result<Self, PushError> {
        Ok(Self {
            server: server.to_string(),
            registrations: Registrations::open(path)?,
            devices,
            sender: WebPush::new(&CoreProxy::default())?,
        })
    }

    /// 发推送走核心自己的代理（ADR-0023）。
    pub fn with_proxy(mut self, proxy: &CoreProxy) -> Result<Self, PushError> {
        self.sender = WebPush::new(proxy)?;
        Ok(self)
    }

    /// 登记或更新一台设备的推送订阅。再次登记时 pushkey 不变。
    pub fn register(
        &self,
        device: EndpointId,
        subscription: Subscription,
    ) -> Result<Registration, PushError> {
        let registration = self.registrations.upsert(device, subscription)?;
        tracing::info!(device = %device.fmt_short(), endpoint = %registration.subscription.endpoint_host(), "登记推送");
        Ok(registration)
    }

    /// 删掉一台设备的推送登记：设备取消登记，或者被吊销。返回它之前是否登记过。
    pub fn unregister(&self, device: &EndpointId) -> Result<bool, PushError> {
        let removed = self.registrations.remove(device)?;
        if removed {
            tracing::info!(device = %device.fmt_short(), "删除推送登记");
        }
        Ok(removed)
    }

    pub fn registration(&self, device: &EndpointId) -> Option<Registration> {
        self.registrations.get(device)
    }

    /// 处理 Tuwunel 的一次通知，返回不再有效的 pushkey（推送网关响应的 `rejected`）。推送服务暂时不可用时返回
    /// [`PushError::Unavailable`]，推送代理据此回 502，Tuwunel 稍后重试。
    pub async fn notify(&self, notification: &Notification) -> Result<Vec<String>, PushError> {
        let payload = notification.payload(&self.server);
        let mut rejected = Vec::new();
        let mut unavailable = None;
        for device in &notification.devices {
            let Some(registration) = self.registrations.find(&device.pushkey) else {
                rejected.push(device.pushkey.clone());
                continue;
            };
            if !self.devices.contains(&registration.device) {
                tracing::warn!(device = %registration.device.fmt_short(), "设备已吊销，不再推送");
                self.forget(&registration);
                rejected.push(device.pushkey.clone());
                continue;
            }
            // 只有计数的通知不转发：App 不靠它更新什么，转发只会白白唤醒手机。
            let Some(payload) = &payload else { continue };
            let plaintext = serde_json::to_vec(payload).expect("推送内容总能序列化");
            match self
                .deliver(
                    &registration,
                    plaintext,
                    notification.urgency(),
                    &payload.event_id,
                )
                .await
            {
                Ok(true) => {}
                Ok(false) => rejected.push(device.pushkey.clone()),
                Err(e) => unavailable = Some(e),
            }
        }
        match unavailable {
            Some(e) => Err(e),
            None => Ok(rejected),
        }
    }

    /// 把一条新的联系人请求推给一个人的所有设备（`user` 为空是主人，ADR-0045）。核心自己发起，没有 Tuwunel 等着
    /// 重试：推送服务暂时不可用时记日志，请求本身在“等你确认”列表里还在。
    pub async fn notify_contact_request(
        &self,
        user: Option<&str>,
        request: wonton_protocol::ContactRequestPush,
    ) {
        let payload = DevicePayload {
            server: self.server.clone(),
            room_id: String::new(),
            event_id: String::new(),
            sender: Some(request.user_id.clone()),
            sender_display_name: request.display_name.clone(),
            room_name: None,
            prio: None,
            contact_request: Some(request),
        };
        let plaintext = serde_json::to_vec(&payload).expect("推送内容总能序列化");
        for device in self.devices_of(user) {
            let Some(registration) = self.registrations.get(&device.id) else {
                continue;
            };
            if let Err(e) = self
                .deliver(&registration, plaintext.clone(), "high", "contact_request")
                .await
            {
                tracing::warn!(device = %device.id.fmt_short(), error = %e, "联系人请求的推送没发出去");
            }
        }
    }

    /// 发给一台设备。返回订阅是否还有效。
    async fn deliver(
        &self,
        registration: &Registration,
        plaintext: Vec<u8>,
        urgency: &'static str,
        event: &str,
    ) -> Result<bool, PushError> {
        let device = registration.device.fmt_short();
        let delivery = self
            .sender
            .send(&registration.subscription, plaintext, urgency)
            .await;
        match delivery {
            Ok(Delivery::Accepted) => {
                tracing::info!(%device, %event, urgency, "推送已交给推送服务");
                Ok(true)
            }
            Ok(Delivery::Gone) => {
                tracing::warn!(%device, "推送服务说订阅已失效，删除这台设备的推送登记");
                self.forget(registration);
                Ok(false)
            }
            // 重试也没用，不让 Tuwunel 重发；订阅本身可能还好，先不删。
            Ok(Delivery::Refused { status, reason }) => {
                tracing::error!(%device, status, %reason, "推送服务拒绝了推送");
                Ok(true)
            }
            Err(PushError::Unavailable(message)) => {
                tracing::warn!(%device, %message, "推送服务暂时不可用，等 Tuwunel 重试");
                Err(PushError::Unavailable(message))
            }
            // 登记时校验过，到这里说明登记表里的订阅坏了：删掉，让 App 重新登记。
            Err(e) => {
                tracing::error!(%device, error = %e, "推送订阅不可用，删除这台设备的推送登记");
                self.forget(registration);
                Ok(false)
            }
        }
    }

    /// 一个人名下的已配对设备（`person` 为空是主人，ADR-0045）。
    fn devices_of(&self, person: Option<&str>) -> Vec<Device> {
        match person {
            Some(person) => self.devices.of_user(person),
            None => self
                .devices
                .list()
                .into_iter()
                .filter(|device| device.user.is_none())
                .collect(),
        }
    }

    /// 这个人（`None` 是主人）名下有没有已配对的设备：没有就没人能回答发给他的确认卡片。
    pub fn has_devices(&self, person: Option<&str>) -> bool {
        !self.devices_of(person).is_empty()
    }

    /// 发给一个人（`None` 是主人）名下的已配对设备：确认卡片直推（ADR-0016）。卡片不在任何 Matrix
    /// 房间里，直推是它到设备的唯一通道；明文里是 `confirmation` 段，加密同消息推送。返回发到的设备数；
    /// 一台都没发成时给原因（设备没配对、没登记推送或推送服务不可用），个别设备失败只记日志。
    pub async fn send_confirmation(
        &self,
        person: Option<&str>,
        card: &DeviceConfirmationPush,
    ) -> Result<usize, PushError> {
        let who = person.unwrap_or("owner");
        // 字段名是和 App 之间的契约（wonton-protocol 的 `DeviceConfirmationPush`），App 靠 `confirmation`
        // 认出这不是房间里的消息。
        let plaintext = serde_json::to_vec(&json!({
            "server": self.server,
            "confirmation": card,
        }))
        .expect("确认推送总能序列化");
        let devices = self.devices_of(person);
        if devices.is_empty() {
            return Err(PushError::Invalid(format!(
                "{who} 名下没有已配对的设备，确认卡片推不出去"
            )));
        }
        let mut sent = 0usize;
        let mut unavailable = None;
        for device in &devices {
            let Some(registration) = self.registrations.get(&device.id) else {
                tracing::warn!(person = who, device = %device.id.fmt_short(), "设备没登记推送，确认卡片推不到它");
                continue;
            };
            if !self.devices.contains(&registration.device) {
                tracing::warn!(device = %registration.device.fmt_short(), "设备已吊销，不再推送");
                self.forget(&registration);
                continue;
            }
            match self
                .deliver(&registration, plaintext.clone(), "high", &card.id)
                .await
            {
                Ok(true) => sent += 1,
                Ok(false) => {}
                Err(e) => unavailable = Some(e),
            }
        }
        match (sent, unavailable) {
            (0, Some(e)) => Err(e),
            (0, None) => Err(PushError::Invalid(format!(
                "{who} 的设备都没登记推送，确认卡片推不出去"
            ))),
            _ => Ok(sent),
        }
    }

    /// 删不掉也照样告诉 Tuwunel 删 pusher：推送前会再查设备登记表，留在表里的订阅不会再被用到。
    fn forget(&self, registration: &Registration) {
        if let Err(e) = self.registrations.remove(&registration.device) {
            tracing::error!(device = %registration.device.fmt_short(), error = %e, "删除推送登记失败");
        }
    }
}

#[cfg(test)]
mod tests;
