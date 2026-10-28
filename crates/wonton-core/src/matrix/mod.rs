//! 核心和 Matrix 服务器（Tuwunel）：签发登录 JWT、持有管理账号（ADR-0009）。
//!
//! 核心只经 Tuwunel 的客户端接口和 Synapse 兼容的管理接口和它说话，不读它的数据（ADR-0015）。设备和 Bot
//! 自己用核心签的 JWT 登录；核心的管理账号用来吊销设备、停用账号。

mod admin;
mod api;
pub(crate) mod issuer;

use std::path::Path;
use std::time::Duration;

use hyper::Method;

pub use admin::ADMIN_LOCALPART;
pub use api::{JWT_LOGIN_TYPE, MatrixError};
pub use issuer::LOGIN_TOKEN_TTL_SECS;

use crate::config::MatrixConfig;
use crate::http::HttpClient;
use api::Api;
use issuer::Issuer;

/// 等 Tuwunel 就绪的上限：升级后首次启动可能在迁移数据库。
const READY_TIMEOUT: Duration = Duration::from_secs(90);

pub struct Matrix {
    api: Api,
    server_name: String,
    issuer: Issuer,
    admin_token: String,
}

impl Matrix {
    /// 等 Tuwunel 就绪，准备好管理账号。管理令牌存在 `state_dir` 里。
    pub async fn connect(
        config: &MatrixConfig,
        state_dir: &Path,
        http: HttpClient,
    ) -> Result<Self, MatrixError> {
        let issuer = Issuer::from_pem(&config.jwt_key).map_err(MatrixError::Invalid)?;
        let api = Api::new(http, &config.url);
        api.wait_ready(READY_TIMEOUT).await?;
        let path = state_dir.join("matrix-admin-token");
        let admin_token =
            admin::admin_token(&api, &issuer, &config.registration_secret, &path).await?;
        let matrix = Self {
            api,
            server_name: config.server_name.clone(),
            issuer,
            admin_token,
        };
        matrix.check_admin().await?;
        Ok(matrix)
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// 客户端接口的内部地址 `主机:端口`，网关把请求转发到这里。
    pub fn authority(&self) -> &str {
        self.api.base().trim_start_matches("http://")
    }

    /// `@<localpart>:<server_name>`。
    pub fn user_id(&self, localpart: &str) -> String {
        format!("@{localpart}:{}", self.server_name)
    }

    /// 核心管理账号的用户 ID。
    pub fn admin_user_id(&self) -> String {
        self.user_id(admin::ADMIN_LOCALPART)
    }

    /// 给 `localpart` 签一个登录令牌，[`LOGIN_TOKEN_TTL_SECS`] 秒内有效。
    pub fn login_token(&self, localpart: &str) -> String {
        self.issuer.login_token(localpart)
    }

    /// 确认管理账号真是服务器管理员。账号若先被 JWT 登录建成了普通账号，注册会报 `M_USER_IN_USE`，再登录拿到的
    /// 令牌调不了管理接口：启动时就停下，不等到要吊销设备时才失败。
    async fn check_admin(&self) -> Result<(), MatrixError> {
        let path = format!("/_synapse/admin/v2/users/{}/devices", self.admin_user_id());
        self.admin(Method::GET, &path, None, "确认核心的管理账号是服务器管理员")
            .await
            .map(|_| ())
    }

    async fn admin(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
        context: &str,
    ) -> Result<serde_json::Value, MatrixError> {
        self.api
            .json(method, path, Some(&self.admin_token), body, context)
            .await
    }
}

#[cfg(test)]
pub(crate) mod tests;
