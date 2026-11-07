//! 以服务器为准查设备密钥，给客户端核心的集成测试等“对方设备的密钥真的上传了”再动手。

use crate::{Error, MatrixSession, OwnedDeviceId, Result, UserId};
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
}
