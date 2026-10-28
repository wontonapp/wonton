//! 登录与会话：用核心签发的 JWT 登录 Tuwunel（ADR-0009），把登录状态存下来，重启后恢复。

use std::path::PathBuf;

use futures_util::{Stream, stream};
use matrix_sdk::{
    Client, ClientBuildError, SessionChange,
    authentication::matrix::MatrixSession as SdkSession,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    ruma::{DeviceId, OwnedDeviceId, OwnedUserId, UserId, serde::JsonObject},
    sliding_sync::VersionBuilder,
};
use matrix_sdk_ui::sync_service::SyncService;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;

use crate::{Error, Result};

/// Tuwunel 的 JWT 登录类型。
const JWT_LOGIN_TYPE: &str = "org.matrix.login.jwt";

/// 登录参数。`Debug` 不输出代理的会话凭据和存储口令。
#[derive(Clone)]
pub struct SessionConfig {
    /// Tuwunel 的地址；经隧道时是客户端核心本地代理的地址。
    pub homeserver_url: String,
    /// 设备在服务器上的显示名。
    pub device_name: String,
    /// 本地存储。
    pub store: StoreConfig,
    /// HTTP 代理，如 `http://用户名:密码@127.0.0.1:端口`。经隧道访问时是客户端核心本地代理的地址和本次
    /// 启动的会话凭据；直连服务器时为空。
    pub proxy: Option<String>,
}

/// 本地存储。
#[derive(Clone)]
pub enum StoreConfig {
    /// 只放在内存里，进程退出即丢失，用于测试和一次性工具。
    InMemory,
    /// SQLite 持久化会话状态、加密密钥和事件、媒体缓存。
    Sqlite {
        /// 存储目录，每个账号一个。
        path: PathBuf,
        /// 加密本地存储的口令；`None` 表示不加密。
        passphrase: Option<String>,
    },
}

impl std::fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConfig")
            .field("homeserver_url", &self.homeserver_url)
            .field("device_name", &self.device_name)
            .field("store", &self.store)
            .field("proxy", &self.proxy.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl std::fmt::Debug for StoreConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InMemory => f.write_str("InMemory"),
            Self::Sqlite { path, passphrase } => f
                .debug_struct("Sqlite")
                .field("path", path)
                .field("passphrase", &passphrase.as_ref().map(|_| "<redacted>"))
                .finish(),
        }
    }
}

/// 会话事件，客户端核心据此持久化新令牌，或提示用户重新配对。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// 访问令牌已用刷新令牌换新。
    TokensRefreshed,
    /// 服务器不再接受当前令牌。`soft_logout` 为假表示设备已被删除，只能重新配对。
    TokenRejected {
        /// 是否软登出：设备还在，换一个令牌即可恢复。
        soft_logout: bool,
    },
}

/// 存下来的登录状态：用户、设备和令牌。重启后交给 [`MatrixSession::restore`]，不用重新登录。
///
/// 令牌等同于登录凭据，要和设备密钥一样保管；`Debug` 不输出令牌。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SavedSession(SdkSession);

impl SavedSession {
    /// 这份登录状态属于的用户。
    pub fn user_id(&self) -> &UserId {
        &self.0.meta.user_id
    }

    /// 这份登录状态属于的设备。
    pub fn device_id(&self) -> &DeviceId {
        &self.0.meta.device_id
    }
}

/// 一个已登录设备的 Matrix 会话。
pub struct MatrixSession {
    pub(crate) client: Client,
    pub(crate) sync: SyncService,
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
}

impl MatrixSession {
    /// 用核心签发的 JWT 登录，在服务器上新建一个设备。
    ///
    /// 账号在首次登录时由 Tuwunel 自动创建。登录时申请刷新令牌：访问令牌按服务器的
    /// `access_token_ttl` 过期，SDK 遇到过期会自动换新，并发出 [`SessionEvent::TokensRefreshed`]。
    pub async fn login_with_jwt(config: &SessionConfig, jwt: &str) -> Result<Self> {
        let client = build_client(config).await?;
        let data = JsonObject::from_iter([("token".to_owned(), jwt.into())]);
        let response = client
            .matrix_auth()
            .login_custom(JWT_LOGIN_TYPE, data)
            .map_err(|error| Error::InvalidInput(error.to_string()))?
            .initial_device_display_name(&config.device_name)
            .request_refresh_token()
            .await?;
        Self::with_client(client, response.user_id, response.device_id).await
    }

    /// 用存下来的登录状态恢复会话，不经服务器登录。
    ///
    /// 用 SQLite 存储时要传登录时的同一个目录：设备密钥、交叉签名和密钥备份都在里面。令牌已经失效的话，
    /// 恢复本身照样成功，第一次请求时才报错，并发出 [`SessionEvent::TokenRejected`]。
    pub async fn restore(config: &SessionConfig, saved: SavedSession) -> Result<Self> {
        let client = build_client(config).await?;
        let user_id = saved.0.meta.user_id.clone();
        let device_id = saved.0.meta.device_id.clone();
        client.restore_session(saved.0).await?;
        Self::with_client(client, user_id, device_id).await
    }

    async fn with_client(
        client: Client,
        user_id: OwnedUserId,
        device_id: OwnedDeviceId,
    ) -> Result<Self> {
        let sync = SyncService::builder(client.clone())
            .build()
            .await
            .map_err(|error| Error::from(&error))?;
        Ok(Self {
            client,
            sync,
            user_id,
            device_id,
        })
    }

    /// 当前的登录状态，用来存到本机。访问令牌刷新后（[`SessionEvent::TokensRefreshed`]）要重新取一次存下。
    pub fn saved(&self) -> Option<SavedSession> {
        self.client.matrix_auth().session().map(SavedSession)
    }

    /// 本设备登录的用户。
    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    /// 本设备在服务器上的 ID，核心吊销设备时用它。
    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    /// 会话事件流。只包含订阅之后发生的事件。
    pub fn session_events(&self) -> impl Stream<Item = SessionEvent> + use<> {
        stream::unfold(
            self.client.subscribe_to_session_changes(),
            |mut receiver| async move {
                loop {
                    match receiver.recv().await {
                        Ok(SessionChange::TokensRefreshed) => {
                            return Some((SessionEvent::TokensRefreshed, receiver));
                        }
                        Ok(SessionChange::UnknownToken(data)) => {
                            let event = SessionEvent::TokenRejected {
                                soft_logout: data.soft_logout,
                            };
                            return Some((event, receiver));
                        }
                        // SDK 的通道只保留最新一条：落后时跳过的是旧事件，最新的一条照样收到。
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => return None,
                    }
                }
            },
        )
    }

    /// 设置本账号的显示名。核心签发 JWT 登录时自动建的账号没有显示名，别人那里显示的是用户 ID。
    pub async fn set_display_name(&self, name: &str) -> Result<()> {
        self.client.account().set_display_name(Some(name)).await?;
        Ok(())
    }

    /// 退出登录：停止同步，服务器删除这个设备。
    pub async fn logout(self) -> Result<()> {
        self.sync.stop().await;
        self.client.logout().await?;
        Ok(())
    }
}

async fn build_client(config: &SessionConfig) -> Result<Client> {
    let mut builder = Client::builder()
        .homeserver_url(&config.homeserver_url)
        // 先查服务器是否支持 MSC4186，不支持就在这里失败，而不是等到同步时才发现。
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .handle_refresh_tokens()
        .with_encryption_settings(EncryptionSettings {
            // 登录后自动建交叉签名身份（账号还没有时）；服务器要支持 MSC3967，首次上传不要求再次认证。
            auto_enable_cross_signing: true,
            // 解密失败时再去密钥备份里找密钥，和 Element X 一样。
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            // 密钥备份随恢复一起建（[`MatrixSession::enable_recovery`]），不在登录时自动建。
            auto_enable_backups: false,
        });
    if let Some(proxy) = &config.proxy {
        builder = builder.proxy(proxy);
    }
    let builder = match &config.store {
        StoreConfig::InMemory => builder,
        StoreConfig::Sqlite { path, passphrase } => {
            builder.sqlite_store(path, passphrase.as_deref())
        }
    };
    builder.build().await.map_err(client_build_error)
}

fn client_build_error(error: ClientBuildError) -> Error {
    match error {
        ClientBuildError::Http(http) => (&http).into(),
        ClientBuildError::Url(source) => Error::InvalidInput(format!("服务器地址无效：{source}")),
        ClientBuildError::SlidingSyncVersion(source) => Error::NotReady(format!(
            "服务器不支持 Simplified Sliding Sync（MSC4186）：{source}"
        )),
        other => Error::Upstream(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 代理地址里带着本地代理的会话凭据，存储口令能解开本地的密钥：都不能随 `Debug` 进日志。
    #[test]
    fn debug_hides_proxy_credentials_and_the_store_passphrase() {
        let config = SessionConfig {
            homeserver_url: "http://matrix.wonton.localhost".to_owned(),
            device_name: "手机".to_owned(),
            store: StoreConfig::Sqlite {
                path: PathBuf::from("/data/matrix/store"),
                passphrase: Some("store-passphrase".to_owned()),
            },
            proxy: Some("http://wonton:proxy-password@127.0.0.1:49152".to_owned()),
        };
        let printed = format!("{config:?}");
        assert!(!printed.contains("proxy-password"), "{printed}");
        assert!(!printed.contains("store-passphrase"), "{printed}");
        assert!(printed.contains("matrix.wonton.localhost"), "{printed}");
    }
}
