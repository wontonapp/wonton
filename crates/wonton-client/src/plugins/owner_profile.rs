//! 你的资料（ADR-0030）：设置页和安装页读写显示名和邮箱；刚配对、资料还没填时让壳弹一次填资料的页面。

use std::sync::Arc;
use std::sync::atomic::Ordering;

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
        self.dismiss_owner_profile_prompt();
        Ok(saved)
    }

    /// 这次连上的是刚配对的服务器：资料还没填就提示一次。取不到时不提示，也不算错：要用资料的时候安装页还会让主人填。
    pub(super) fn prompt_owner_profile_if_empty(self: &Arc<Self>) {
        let generation = self.generation.load(Ordering::SeqCst);
        let link = self.clone();
        self.runtime.spawn(async move {
            let profile = match link.owner_profile().await {
                Ok(profile) => profile,
                Err(e) => {
                    tracing::info!("取不到资料，这次不提示：{e}");
                    return;
                }
            };
            if link.is_current(generation)
                && profile.display_name.is_none()
                && profile.email.is_none()
            {
                link.view
                    .send_modify(|view| view.owner_profile_prompt = true);
            }
        });
    }
}
