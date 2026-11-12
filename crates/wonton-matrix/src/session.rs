//! 登录与会话：用核心签发的 JWT 登录 Tuwunel（ADR-0009），把登录状态存下来，重启后恢复。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{Stream, stream};
use matrix_sdk::{
    Client, ClientBuildError, ClientBuilder, SessionChange, SessionMeta, SessionTokens,
    ThreadingSupport,
    authentication::matrix::MatrixSession as SdkSession,
    cross_process_lock::CrossProcessLockConfig,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    reqwest,
    ruma::{DeviceId, OwnedDeviceId, OwnedRoomId, OwnedUserId, RoomId, UserId, serde::JsonObject},
    sliding_sync::VersionBuilder,
    store::RoomLoadSettings,
};
use matrix_sdk_ui::sync_service::SyncService;
use tokio::{
    sync::{Notify, broadcast::error::RecvError},
    task::JoinHandle,
};

use crate::rooms::RoomSubscriptions;
pub use crate::saved::SavedSession;
use crate::{Error, Result, send_queue};

/// Tuwunel 的 JWT 登录类型。
const JWT_LOGIN_TYPE: &str = "org.matrix.login.jwt";

/// 房间列表的两个连接号，轮换着用：上次用哪个记在存储目录下的文件里，这次用另一个，服务端那边的旧会话
/// 就不会挡住新连接。都是 13 个字符，SDK 只收 16 个以内，测试守着。
const ROOM_LIST_CONN_IDS: [&str; 2] = ["room-list-a", "room-list-b"];
/// 记上次用的连接号的文件名，和 Matrix 存储放在同一个目录（归客户端核心所有）。
const ROOM_LIST_CONN_FILE: &str = "room-list-conn";

/// 这次用哪个连接号，并记下来。没有记录（第一次启动）从第一个开始；连着两次都在 30 秒内崩溃时，第三个实例会
/// 轮回到第一个连接号，死掉那条长轮询可能还挂在上面，最多再等它一轮（30 秒），不会卡死。内存存储没有目录，
/// 按进程里的调用次数轮换，够一个进程里几个会话分开。
async fn next_room_list_conn_id(store: &StoreConfig) -> &'static str {
    let StoreConfig::Sqlite { path, .. } = store else {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        return ROOM_LIST_CONN_IDS[NEXT.fetch_add(1, Ordering::Relaxed) % ROOM_LIST_CONN_IDS.len()];
    };
    let previous = tokio::fs::read_to_string(path.join(ROOM_LIST_CONN_FILE))
        .await
        .unwrap_or_default();
    let next = if previous.trim() == ROOM_LIST_CONN_IDS[0] {
        ROOM_LIST_CONN_IDS[1]
    } else {
        ROOM_LIST_CONN_IDS[0]
    };
    if let Err(error) = tokio::fs::write(path.join(ROOM_LIST_CONN_FILE), next).await {
        tracing::warn!("could not record the room list connection id: {error}");
    }
    next
}

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

/// 一个已登录设备的 Matrix 会话。
pub struct MatrixSession {
    pub(crate) client: Client,
    /// 推送取消息时要和它共用加密同步（`NotificationProcessSetup::SingleProcess`）。
    pub(crate) sync: Arc<SyncService>,
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    /// 发送出错以后接着发（[`send_queue`](crate::send_queue)），会话释放时停止。
    sending: JoinHandle<()>,
    pub(crate) subscriptions: tokio::sync::Mutex<RoomSubscriptions>,
    pub(crate) verifications: Arc<crate::verification::concurrency::Registry>,
    /// 叫醒等着重开发送队列的任务，见 [`MatrixSession::resume`]。
    pub(crate) resume_sending: Arc<Notify>,
    /// 叫醒等着重启同步的任务，见 [`MatrixSession::resume`]。
    pub(crate) resume_sync: Notify,
    /// 本会话第一次看到每个房间的时刻（Unix 毫秒）。Tuwunel 2.0 的邀请剥离状态不带时间戳，邀请的
    /// 建房时间读不到时用它代替；只用来排序，不持久化，重启后按列表顺序回退。
    invited_seen: std::sync::Mutex<std::collections::BTreeMap<OwnedRoomId, u64>>,
    /// 叫醒房间列表重算一遍：话题回执只改话题的未读，SDK 不当成房间的变化（ADR-0093 §4）。
    pub(crate) rooms_changed: Arc<Notify>,
}

impl Drop for MatrixSession {
    fn drop(&mut self) {
        self.sending.abort();
    }
}

impl MatrixSession {
    /// 用核心签发的 JWT 登录，在服务器上新建一个设备。
    ///
    /// 账号在首次登录时由 Tuwunel 自动创建。登录时申请刷新令牌：访问令牌按服务器的
    /// `access_token_ttl` 过期，SDK 遇到过期会自动换新，并发出 [`SessionEvent::TokensRefreshed`]。
    pub async fn login_with_jwt(config: &SessionConfig, jwt: &str) -> Result<Self> {
        let client = build_client(config, VersionBuilder::DiscoverNative).await?;
        let data = JsonObject::from_iter([("token".to_owned(), jwt.into())]);
        let response = client
            .matrix_auth()
            .login_custom(JWT_LOGIN_TYPE, data)
            .map_err(|error| Error::InvalidInput(error.to_string()))?
            .initial_device_display_name(&config.device_name)
            .request_refresh_token()
            .await
            // Tuwunel 拒绝 JWT（过期、签发方不对、伪造）回 403 `M_FORBIDDEN`：登录被拒说的是身份。
            .map_err(|error| Error::from(error).into_unauthorized())?;
        Self::with_client(client, response.user_id, response.device_id, config).await
    }

    /// 用核心交给 Bot 的访问令牌恢复设备；令牌只用于本次进程，不写进 Matrix 状态库。
    ///
    /// 先通过 `whoami` 得到令牌绑定的用户和设备，再把完整会话交给 SDK 恢复加密存储。重装插件时 Bot 会换一个
    /// 设备，旧设备的加密存储不能直接套给新设备；设备变了就丢掉旧存储，从新设备重新开始。
    pub async fn login_with_access_token(config: &SessionConfig, token: &str) -> Result<Self> {
        if token.trim().is_empty() {
            return Err(Error::InvalidInput("Matrix 访问令牌为空".to_owned()));
        }
        let (user_id, device_id) = access_token_identity(config, token).await?;
        let marker = reconcile_store(config, &device_id).await?;
        let client = build_client(config, VersionBuilder::DiscoverNative).await?;
        client
            .matrix_auth()
            .restore_session(
                SdkSession {
                    meta: SessionMeta {
                        user_id: user_id.clone(),
                        device_id: device_id.clone(),
                    },
                    tokens: SessionTokens {
                        access_token: token.to_owned(),
                        refresh_token: None,
                    },
                },
                RoomLoadSettings::default(),
            )
            .await
            .map_err(|error| Error::Upstream(format!("恢复 Matrix 会话失败：{error}")))?;
        let session = Self::with_client(client, user_id, device_id.clone(), config).await?;
        if let Some(marker) = marker {
            tokio::fs::write(&marker, device_id.as_str())
                .await
                .map_err(|error| Error::Upstream(format!("保存 Matrix 设备标记失败：{error}")))?;
        }
        Ok(session)
    }

    /// 用存下来的登录状态恢复会话，不经服务器登录。
    ///
    /// 用 SQLite 存储时要传登录时的同一个目录：设备密钥、交叉签名和密钥备份都在里面。令牌已经失效的话，
    /// 恢复本身照样成功，第一次请求时才报错，并发出 [`SessionEvent::TokenRejected`]。
    pub async fn restore(config: &SessionConfig, saved: SavedSession) -> Result<Self> {
        // 不查服务器版本：冷启动没网也要能打开本地的房间和消息。登录时已经查过这台服务器。
        let client = build_client(config, VersionBuilder::Native).await?;
        let user_id = saved.user_id().to_owned();
        let device_id = saved.device_id().to_owned();
        saved.restore_into(&client).await?;
        Self::with_client(client, user_id, device_id, config).await
    }

    async fn with_client(
        client: Client,
        user_id: OwnedUserId,
        device_id: OwnedDeviceId,
        config: &SessionConfig,
    ) -> Result<Self> {
        // 每次实例轮换房间列表的连接号。服务器按（设备，连接号）一条一条地处理 Sliding Sync：重启后沿用
        // 同一个连接号，新请求就排在死掉的那条还没结束的 30 秒长轮询后面，起步干等半分钟；换一个就不排队了。
        // 也不开 `share_pos`：位置按连接号存，轮换后读回来的是两轮前的，服务端早不认，还是要先挂一轮再回
        // `M_UNKNOWN_POS`，不如省掉。
        // 同一个实例内的重启（[`Self::keep_syncing`]）连接号和内存里的 `pos` 都不变。
        let sync = SyncService::builder(client.clone())
            .with_room_list_conn_id(next_room_list_conn_id(&config.store).await.to_owned())
            .with_share_pos(false)
            .build()
            .await
            .map_err(|error| Error::from(&error))?;
        let sync = Arc::new(sync);
        // 上传附件时时间线本地回显带上进度（#92）：开关是 client 级的，每次上传开始时读一次。
        client.send_queue().enable_upload_progress(true);
        let resume_sending = Arc::new(Notify::new());
        let sending = tokio::spawn(send_queue::keep_sending(
            client.clone(),
            resume_sending.clone(),
        ));
        Ok(Self {
            client,
            sync,
            user_id,
            device_id,
            sending,
            resume_sending,
            resume_sync: Notify::new(),
            rooms_changed: Arc::new(Notify::new()),
            subscriptions: tokio::sync::Mutex::default(),
            verifications: Arc::default(),
            invited_seen: std::sync::Mutex::default(),
        })
    }

    /// 本会话第一次看到 `room_id` 的时刻；第一次问时记成现在。
    pub(crate) async fn invited_seen_ms(&self, room_id: &RoomId) -> u64 {
        let mut seen = self.invited_seen.lock().expect("invited_seen 锁");
        *seen.entry(room_id.to_owned()).or_insert_with(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        })
    }

    /// 当前的登录状态，用来存到本机。访问令牌刷新后（[`SessionEvent::TokensRefreshed`]）要重新取一次存下。
    pub fn saved(&self) -> Option<SavedSession> {
        SavedSession::of(&self.client)
    }

    /// 本设备登录的用户。
    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    /// 底层 SDK 客户端：本 crate 的封装之外的操作（如自定义状态事件、账号数据）用它。
    pub fn client(&self) -> &Client {
        &self.client
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

async fn access_token_identity(
    config: &SessionConfig,
    token: &str,
) -> Result<(OwnedUserId, OwnedDeviceId)> {
    let mut http = reqwest::Client::builder();
    if let Some(proxy) = &config.proxy {
        http = http.proxy(
            reqwest::Proxy::all(proxy)
                .map_err(|error| Error::InvalidInput(format!("Matrix 代理地址无效：{error}")))?,
        );
    }
    let url = format!(
        "{}/_matrix/client/v3/account/whoami",
        config.homeserver_url.trim_end_matches('/')
    );
    let response = http
        .build()
        .map_err(|error| Error::Transport(format!("建立 Matrix HTTP 客户端失败：{error}")))?
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| Error::Transport(format!("查询 Matrix 令牌身份失败：{error}")))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| Error::Transport(format!("读取 Matrix 身份响应失败：{error}")))?;
    let body: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        Error::Upstream(format!(
            "Matrix 身份响应不是 JSON（HTTP {status}）：{error}"
        ))
    })?;
    if !status.is_success() {
        let message = body["error"]
            .as_str()
            .unwrap_or("Matrix 拒绝了这个访问令牌")
            .to_owned();
        return Err(match status.as_u16() {
            401 | 403 => Error::Unauthorized {
                errcode: body["errcode"].as_str().unwrap_or_default().to_owned(),
                message,
            },
            400 => Error::InvalidInput(message),
            _ => Error::Upstream(format!("HTTP {status}: {message}")),
        });
    }
    let user_id = body["user_id"]
        .as_str()
        .ok_or_else(|| Error::Upstream("Matrix 身份响应缺少 user_id".to_owned()))?
        .parse::<OwnedUserId>()
        .map_err(|error| Error::InvalidInput(format!("Matrix user_id 无效：{error}")))?;
    let device_id: OwnedDeviceId = body["device_id"]
        .as_str()
        .ok_or_else(|| Error::Upstream("Matrix 身份响应缺少 device_id".to_owned()))?
        .into();
    Ok((user_id, device_id))
}

/// 设备变了就丢掉旧 SQLite 存储，返回要写入新设备号的标记文件。
async fn reconcile_store(config: &SessionConfig, device_id: &DeviceId) -> Result<Option<PathBuf>> {
    let StoreConfig::Sqlite { path, .. } = &config.store else {
        return Ok(None);
    };
    let marker = path.with_extension("device");
    let previous = match tokio::fs::read_to_string(&marker).await {
        Ok(previous) => Some(previous),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(Error::Upstream(format!(
                "读取 Matrix 设备标记失败：{error}"
            )));
        }
    };
    let old_store = path.exists()
        && previous
            .as_deref()
            .is_none_or(|value| value.trim() != device_id.as_str());
    if old_store {
        match tokio::fs::remove_dir_all(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(Error::Upstream(format!(
                    "清理旧设备的 Matrix 存储失败：{error}"
                )));
            }
        }
    }
    Ok(Some(marker))
}

async fn build_client(config: &SessionConfig, versions: VersionBuilder) -> Result<Client> {
    client_builder(&config.store, config.proxy.as_deref(), true)
        .homeserver_url(&config.homeserver_url)
        .sliding_sync_version_builder(versions)
        .build()
        .await
        .map_err(client_build_error)
}

/// 除了服务器地址以外的客户端设置，登录（[`crate::login`]）和会话共用。`cross_signing` 为假时登录后不自动建交叉签名：
/// 用现成账号登录时由之后恢复的会话来建，登录用的客户端用完就停，不在半路上传一半。
pub(crate) fn client_builder(
    store: &StoreConfig,
    proxy: Option<&str>,
    cross_signing: bool,
) -> ClientBuilder {
    let mut builder = Client::builder()
        .handle_refresh_tokens()
        // 事件缓存按话题分开记、主对话的未读不算话题回复，话题的未读按话题回执另算（ADR-0093 §4）。
        // 不订阅话题（MSC4306），所有话题都照常同步。
        .with_threading_support(ThreadingSupport::Enabled {
            with_subscriptions: false,
        })
        .with_encryption_settings(EncryptionSettings {
            // 登录后自动建交叉签名身份（账号还没有时）；服务器要支持 MSC3967，首次上传不要求再次认证。
            auto_enable_cross_signing: cross_signing,
            // 解密失败时再去密钥备份里找密钥，和 Element X 一样。
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            // 密钥备份随恢复一起建（[`MatrixSession::enable_recovery`]），不在登录时自动建。
            auto_enable_backups: false,
        });
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy);
    }
    let path = match store {
        StoreConfig::InMemory => None,
        StoreConfig::Sqlite { path, .. } => Some(path.as_path()),
    };
    builder = builder.cross_process_store_config(CrossProcessLockConfig::multi_process(
        crate::store_holder::holder_for(path),
    ));
    match store {
        StoreConfig::InMemory => builder,
        StoreConfig::Sqlite { path, passphrase } => {
            builder.sqlite_store(path, passphrase.as_deref())
        }
    }
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

    /// SDK 只收 16 个字符以内的实例名，超了在建 `SyncService` 时直接失败。
    #[test]
    fn room_list_conn_ids_fit_the_sdk_limit() {
        for id in ROOM_LIST_CONN_IDS {
            assert!(id.len() <= 16, "{id} 太长");
        }
        assert_ne!(ROOM_LIST_CONN_IDS[0], ROOM_LIST_CONN_IDS[1]);
    }

    /// 连接号两次启动换一个，第三次回到第一个（崩溃循环时最多再等一轮长轮询）。存储里只留一条记录。
    #[tokio::test]
    async fn room_list_conn_ids_rotate() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreConfig::Sqlite {
            path: dir.path().to_owned(),
            passphrase: None,
        };
        let first = next_room_list_conn_id(&store).await;
        let second = next_room_list_conn_id(&store).await;
        let third = next_room_list_conn_id(&store).await;
        assert_ne!(first, second);
        assert_eq!(first, third);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(ROOM_LIST_CONN_FILE)).unwrap(),
            third
        );
    }

    #[tokio::test]
    async fn a_changed_device_clears_the_previous_matrix_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("matrix");
        std::fs::create_dir(&store).unwrap();
        std::fs::write(store.join("state"), b"old").unwrap();
        let marker = store.with_extension("device");
        std::fs::write(&marker, "old-device").unwrap();
        let config = SessionConfig {
            homeserver_url: "http://matrix.wonton.localhost".to_owned(),
            device_name: "Bot".to_owned(),
            store: StoreConfig::Sqlite {
                path: store.clone(),
                passphrase: None,
            },
            proxy: None,
        };
        let device = OwnedDeviceId::from("new-device");
        let written = reconcile_store(&config, &device).await.unwrap();
        assert_eq!(written, Some(marker));
        assert!(!store.exists(), "旧设备的存储应被删除");
    }

    #[tokio::test]
    async fn a_store_without_a_device_marker_is_treated_as_old() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("matrix");
        std::fs::create_dir(&store).unwrap();
        let config = SessionConfig {
            homeserver_url: "http://matrix.wonton.localhost".to_owned(),
            device_name: "Bot".to_owned(),
            store: StoreConfig::Sqlite {
                path: store.clone(),
                passphrase: None,
            },
            proxy: None,
        };
        let device = OwnedDeviceId::from("new-device");
        reconcile_store(&config, &device).await.unwrap();
        assert!(!store.exists());
    }
}
