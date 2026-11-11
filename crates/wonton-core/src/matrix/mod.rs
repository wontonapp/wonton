//! 核心和 Matrix 服务器（Tuwunel）：签发登录 JWT、持有管理账号（ADR-0009）。
//!
//! 核心只经 Tuwunel 的客户端接口和 Synapse 兼容的管理接口和它说话，不读它的数据（ADR-0015）。设备和 Bot
//! 自己用核心签的 JWT 登录；核心的管理账号用来吊销设备、停用账号。

mod admin;
mod api;
mod bot;
mod bot_profile;
pub mod federation_config;
pub mod issuer;
mod notices;
mod session;
use std::path::Path;
use std::time::Duration;

use hyper::Method;
use serde_json::json;

pub use admin::{ADMIN_LOCALPART, SERVER_USER_LOCALPART};
pub use api::{JWT_LOGIN_TYPE, Login, MatrixError};
pub use bot::{Avatar, BotAccount, BotSpec, ProvisionedBot};
pub use federation_config::{FederationBlocks, FederationConfig};
pub use issuer::LOGIN_TOKEN_TTL_SECS;
pub use notices::{Edited, NoticeRoom, Posted};
pub use session::Session;

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

    /// 替 `localpart` 登录一台新设备（首次登录时 Tuwunel 自动建号），不申请刷新令牌。
    pub async fn login(&self, localpart: &str, device_name: &str) -> Result<Login, MatrixError> {
        self.api
            .login(&self.login_token(localpart), device_name)
            .await
    }

    /// 以已登录的用户身份调客户端接口。
    pub fn session(&self, login: &Login) -> Session<'_> {
        Session::new(&self.api, &login.user_id, &login.access_token)
    }

    /// 访问令牌是谁的（`GET /_matrix/client/v3/account/whoami`）：网关认出插件网络里是哪个 Bot 在发请求时用
    /// （ADR-0045），调用方按令牌哈希缓存。
    pub async fn whoami(&self, token: &str) -> Result<String, MatrixError> {
        let value = self
            .api
            .json(
                Method::GET,
                "/_matrix/client/v3/account/whoami",
                Some(token),
                None,
                "辨认访问令牌",
            )
            .await?;
        value["user_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("whoami 没有返回 user_id".to_owned()))
    }

    /// 以核心的管理账号调客户端接口。
    pub(crate) fn core_session(&self) -> Session<'_> {
        Session::new(&self.api, &self.admin_user_id(), &self.admin_token)
    }

    /// 账号存不存在（停用的也算存在）：Synapse 兼容的 `GET /_synapse/admin/v2/users/{user}`，不存在时 Tuwunel 回
    /// 404 `M_NOT_FOUND`。
    pub async fn user_exists(&self, user_id: &str) -> Result<bool, MatrixError> {
        let path = format!("/_synapse/admin/v2/users/{}", api::encode(user_id));
        match self.admin(Method::GET, &path, None, "查询账号").await {
            Ok(_) => Ok(true),
            Err(e) if e.errcode() == "M_NOT_FOUND" => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// 用户现有的设备 ID；用户不存在时为空。
    pub async fn devices(&self, user_id: &str) -> Result<Vec<String>, MatrixError> {
        let path = format!("/_synapse/admin/v2/users/{}/devices", api::encode(user_id));
        let value = match self.admin(Method::GET, &path, None, "列出设备").await {
            Err(e) if e.errcode() == "M_NOT_FOUND" => return Ok(Vec::new()),
            other => other?,
        };
        Ok(value["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d["device_id"].as_str().map(str::to_owned))
            .collect())
    }

    /// 吊销一台设备：它的访问令牌立即作废。设备本来就不在也算成功。
    pub async fn delete_device(&self, user_id: &str, device_id: &str) -> Result<(), MatrixError> {
        let path = format!(
            "/_synapse/admin/v2/users/{}/devices/{}",
            api::encode(user_id),
            api::encode(device_id)
        );
        match self.admin(Method::DELETE, &path, None, "吊销设备").await {
            Err(e) if e.errcode() == "M_NOT_FOUND" => Ok(()),
            other => other.map(|_| ()),
        }
    }

    /// 停用账号：所有设备下线、退出所有房间，之后也不能再登录。不可恢复；账号本来就不在也算成功。
    pub async fn deactivate(&self, user_id: &str) -> Result<(), MatrixError> {
        let path = format!("/_synapse/admin/v1/deactivate/{}", api::encode(user_id));
        let body = json!({"erase": false});
        match self
            .admin(Method::POST, &path, Some(&body), "停用账号")
            .await
        {
            Err(e) if e.errcode() == "M_NOT_FOUND" => Ok(()),
            other => other.map(|_| ()),
        }
    }

    /// 确认管理账号真是服务器管理员。账号若先被 JWT 登录建成了普通账号，注册会报 `M_USER_IN_USE`，再登录拿到的
    /// 令牌调不了管理接口：启动时就停下，不等到要吊销设备时才失败。
    async fn check_admin(&self) -> Result<(), MatrixError> {
        let user = api::encode(&self.admin_user_id());
        let path = format!("/_synapse/admin/v2/users/{user}/devices");
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
pub(crate) mod fake;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod profile_tuwunel_tests;
#[cfg(test)]
mod tests;
