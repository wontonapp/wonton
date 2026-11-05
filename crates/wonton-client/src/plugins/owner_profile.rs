//! 你的资料（ADR-0030）：设置页和安装页读写显示名和邮箱。

use wonton_protocol::{CoreError, OwnerProfile, OwnerProfileUpdate};

use super::PluginsLink;
use super::api::{READ_TIMEOUT, to_core_error};

const PATH: [&str; 1] = ["owner-profile"];

impl PluginsLink {
    pub(crate) async fn owner_profile(&self) -> Result<OwnerProfile, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.get(&PATH, &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    /// 校验在家里的服务器上：名字和邮箱的规则只写在一处，不合规则的原因原样带回来。
    pub(crate) async fn save_owner_profile(
        &self,
        update: OwnerProfileUpdate,
    ) -> Result<OwnerProfile, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let saved = api
            .put(&PATH, &update, READ_TIMEOUT)
            .await
            .map_err(to_core_error)?;
        Ok(saved)
    }
}
