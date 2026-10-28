//! 登录与会话：用核心签发的 JWT 登录 Tuwunel（ADR-0009）。

use std::path::PathBuf;

use futures_util::{Stream, stream};
use matrix_sdk::{
    Client, ClientBuildError, SessionChange,
    ruma::{DeviceId, OwnedDeviceId, OwnedUserId, UserId, serde::JsonObject},
    sliding_sync::VersionBuilder,
};
use matrix_sdk_ui::sync_service::SyncService;
use tokio::sync::broadcast::error::RecvError;

use crate::{Error, Result};

/// Tuwunel 的 JWT 登录类型。
const JWT_LOGIN_TYPE: &str = "org.matrix.login.jwt";

/// 登录参数。
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Tuwunel 的地址；经隧道时是客户端核心本地代理的地址。
    pub homeserver_url: String,
    /// 设备在服务器上的显示名。
    pub device_name: String,
    /// 本地存储。
    pub store: StoreConfig,
}

/// 本地存储。
#[derive(Clone, Debug)]
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
        let sync = SyncService::builder(client.clone())
            .build()
            .await
            .map_err(|error| Error::from(&error))?;
        Ok(Self {
            client,
            sync,
            user_id: response.user_id,
            device_id: response.device_id,
        })
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

    /// 退出登录：停止同步，服务器删除这个设备。
    pub async fn logout(self) -> Result<()> {
        self.sync.stop().await;
        self.client.logout().await?;
        Ok(())
    }
}

async fn build_client(config: &SessionConfig) -> Result<Client> {
    let builder = Client::builder()
        .homeserver_url(&config.homeserver_url)
        // 先查服务器是否支持 MSC4186，不支持就在这里失败，而不是等到同步时才发现。
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .handle_refresh_tokens();
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
