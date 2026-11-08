//! 以服务器为准查设备密钥，给集成测试等“对方设备的密钥真的上传了”再发首条加密消息。

use std::time::Duration;

use crate::{DeviceId, Error, MatrixSession, OwnedDeviceId, Result, UserId};
use matrix_sdk::ruma::api::client::keys::get_keys;

impl MatrixSession {
    /// 这个用户现在在服务器上有哪些设备（`/keys/query`，不看本地 crypto store 的缓存）。
    pub async fn server_device_ids_of(&self, user: &UserId) -> Result<Vec<OwnedDeviceId>> {
        let mut request = get_keys::v3::Request::new();
        request.device_keys.insert(user.to_owned(), Vec::new());
        let response = self
            .client
            .send(request)
            .await
            .map_err(|error| Error::Upstream(format!("keys/query 失败：{error}")))?;
        Ok(response
            .device_keys
            .get(user)
            .map(|devices| devices.keys().cloned().collect())
            .unwrap_or_default())
    }

    /// 房间列表已同步不代表设备密钥已上传；测试发首条加密消息前，以服务器可见性为准。
    pub async fn wait_for_server_device(&self, user: &UserId, device: &DeviceId) {
        let mut last = Vec::new();
        let waiting = async {
            loop {
                last = self
                    .server_device_ids_of(user)
                    .await
                    .expect("查服务器设备密钥");
                if last.iter().any(|id| id.as_str() == device.as_str()) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        if tokio::time::timeout(super::WAIT, waiting).await.is_err() {
            panic!(
                "等 {user} 的设备密钥 {device}：{:?} 内服务器仍不可见，最后的设备：{last:?}",
                super::WAIT
            );
        }
    }
}
