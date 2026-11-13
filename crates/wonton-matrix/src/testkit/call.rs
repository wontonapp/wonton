//! 通话和 widget 测试用（ADR-0096）：另一台设备加入房间里的通话，收发加密的 to-device。

use std::collections::BTreeMap;

use matrix_sdk::deserialized_responses::EncryptionInfo;
use matrix_sdk::ruma::events::AnyToDeviceEvent;
use matrix_sdk::ruma::events::call::member::{
    ActiveFocus, ActiveLivekitFocus, Application, CallApplicationContent, CallMemberEventContent,
    CallMemberStateKey, CallScope, Focus, LivekitFocus,
};
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::to_device::DeviceIdOrAllDevices;
use matrix_sdk::ruma::{DeviceId, RoomId, UserId};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::{Error, MatrixSession, Result};

impl MatrixSession {
    /// 以本设备加入房间里的通话：发 MSC4143 的 `m.call.member`，`state_key` 是 `_<用户>_<设备>_m.call`，
    /// 和 Element Call 的写法一样。
    pub async fn join_call(&self, room_id: &RoomId) -> Result<()> {
        let room = self.room(room_id)?;
        let content = CallMemberEventContent::new(
            Application::Call(CallApplicationContent::new(String::new(), CallScope::Room)),
            self.device_id().to_owned(),
            ActiveFocus::Livekit(ActiveLivekitFocus::new()),
            vec![Focus::Livekit(LivekitFocus::new(
                String::new(),
                "https://livekit.invalid".to_owned(),
            ))],
            None,
            None,
        );
        let key = CallMemberStateKey::new(
            self.user_id().to_owned(),
            Some(format!("{}_m.call", self.device_id())),
            true,
        );
        room.send_state_event_for_key(&key, content).await?;
        Ok(())
    }

    /// 给 `user` 的 `device` 发一条加密的 to-device。对方的设备密钥本机还不知道时没送出去，返回 `false`。
    pub async fn send_encrypted_to_device(
        &self,
        user: &UserId,
        device: &DeviceId,
        event_type: &str,
        content: &Value,
    ) -> Result<bool> {
        let recipients = BTreeMap::from([(
            user.to_owned(),
            vec![DeviceIdOrAllDevices::DeviceId(device.to_owned())],
        )]);
        let raw = serde_json::value::to_raw_value(content)
            .map_err(|error| Error::InvalidInput(error.to_string()))?;
        let failures = self
            .client
            .send_encrypted_to_device(&event_type.into(), recipients, Raw::from_json(raw))
            .await?;
        Ok(failures.is_empty())
    }

    /// 收到的 `event_type` 类型的 to-device：内容，以及是不是加密送来的。
    pub fn to_device_events(&self, event_type: &str) -> mpsc::UnboundedReceiver<(Value, bool)> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let wanted = event_type.to_owned();
        self.client.add_event_handler(
            move |raw: Raw<AnyToDeviceEvent>, encryption: Option<EncryptionInfo>| {
                let sender = sender.clone();
                let wanted = wanted.clone();
                async move {
                    let Ok(event) = raw.deserialize_as_unchecked::<Value>() else {
                        return;
                    };
                    if event["type"] == wanted.as_str() {
                        let _ = sender.send((event["content"].clone(), encryption.is_some()));
                    }
                }
            },
        );
        receiver
    }
}
