//! 这台设备的人（ADR-0045）：设置页和填资料的页面读写自己的资料，家人的设备写的是自己的那条。

use wonton_protocol::{CoreError, Me, OwnerProfileUpdate};

use super::PluginsLink;
use super::api::{READ_TIMEOUT, to_core_error};

const PATH: [&str; 1] = ["me"];

impl PluginsLink {
    pub(crate) async fn me(&self) -> Result<Me, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.get(&PATH, &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    /// 校验在家里服务器的核心里做：名字和邮箱的规则只写一处，不合规则的原因原样带回来。
    pub(crate) async fn save_profile(&self, update: OwnerProfileUpdate) -> Result<Me, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.put(&PATH, &update, READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }
}
