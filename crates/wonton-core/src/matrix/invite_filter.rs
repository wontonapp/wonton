//! 新账号默认开的 MSC4155 邀请过滤（ADR-0051 的“挡垃圾”）。
//!
//! 名单归账号自己的客户端（wonton-client 写 `blocked_users`、`blocked_servers`，别家客户端写的字段原样保留），
//! 核心只在账号第一次登录前把“过滤开着、名单为空”写进去：没有这份账号数据，账号在 Element 这类客户端上就没有过滤。
//! 过滤开着不等于挡住陌生人，空名单全放行，陌生人的邀请照常到客户端，App 把它们放进“请求”；屏蔽谁才进“拒绝”。
//!
//! 事件类型用不稳定名：Tuwunel 只在这个名称上读名单，稳定的 `m.invite_permission_config` 只认 MSC4380 的
//! `default_action`（Tuwunel v2.0.0 `src/service/users/invite_filter.rs` 的 `LISTS_TYPE` 和 `stable_config`）。

use std::collections::BTreeMap;

use hyper::Method;
use serde_json::{Value, json};

use super::{Matrix, MatrixError, api};

pub use wonton_protocol::INVITE_FILTER_EVENT as INVITE_FILTER_TYPE;

/// 临时替账号登录的设备名，写完就吊销。
const DEVICE_NAME: &str = "Wonton 核心（默认过滤）";

/// 默认内容：过滤开着，名单为空。
fn default_content() -> Value {
    json!({"enabled": true})
}

impl Matrix {
    /// 账号还没有邀请过滤的账号数据时写入默认值；已有的不覆盖。
    ///
    /// 账号在第一次 JWT 登录时由 Tuwunel 建（[`Matrix::login`]），所以只能在这里、账号存在之后写。读用管理
    /// 接口，写过以后每次登录只多这一次请求、不进设备列表；要写时才临时登录一台设备，写完吊销。
    pub async fn ensure_default_invite_filter(&self, localpart: &str) -> Result<(), MatrixError> {
        let user_id = self.user_id(localpart);
        if self.user_exists(&user_id).await?
            && self
                .global_account_data(&user_id)
                .await?
                .contains_key(INVITE_FILTER_TYPE)
        {
            return Ok(());
        }
        let login = self.login(localpart, DEVICE_NAME).await?;
        let session = self.session(&login);
        let written = match session.account_data(INVITE_FILTER_TYPE).await {
            Ok(Some(_)) => Ok(()),
            Ok(None) => {
                session
                    .put_account_data(INVITE_FILTER_TYPE, &default_content())
                    .await
            }
            Err(e) => Err(e),
        };
        let revoked = self.delete_device(&user_id, &login.device_id).await;
        written.and(revoked)
    }

    /// 账号的全部全局账号数据（类型 → 内容）：Synapse 兼容的 `GET /_synapse/admin/v1/users/{user}/accountdata`。
    pub(super) async fn global_account_data(
        &self,
        user_id: &str,
    ) -> Result<BTreeMap<String, Value>, MatrixError> {
        let path = format!(
            "/_synapse/admin/v1/users/{}/accountdata",
            api::encode(user_id)
        );
        let value = self.admin(Method::GET, &path, None, "读账号数据").await?;
        Ok(value["account_data"]["global"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(kind, content)| (kind.clone(), content.clone()))
            .collect())
    }
}
