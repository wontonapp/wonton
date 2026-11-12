//! 登录或恢复 Matrix 会话，以及存下的登录状态属于哪台服务器。

use wonton_matrix::{MatrixSession, SessionConfig, StoreConfig};
use wonton_protocol::{ChatFailure, CoreError, DeviceVerification, ProxySettings, RecoveryStatus};

use super::storage::StoredSession;
use super::{ChatLink, failure, login};

/// 本设备在 Matrix 服务器上的显示名。
const DEVICE_NAME: &str = "Wonton App";

impl ChatLink {
    pub(super) async fn sign_in(
        &self,
        proxy: &ProxySettings,
        server_id: String,
    ) -> Result<MatrixSession, (ChatFailure, String)> {
        let config = |homeserver_url: String| SessionConfig {
            homeserver_url,
            device_name: DEVICE_NAME.to_owned(),
            store: StoreConfig::Sqlite {
                path: self.storage.store_dir(),
                passphrase: None,
            },
            proxy: Some(login::proxy_url(proxy)),
        };
        let storage_failure = |e: CoreError| (ChatFailure::Upstream, e.to_string());
        let stored = self.session_on(&server_id).map_err(storage_failure)?;
        // 登录状态要和 SQLite 存储一起用：存储没了还恢复，SDK 会给已有的设备 ID 生成新的设备密钥。存储不在就
        // 重新登录一台新设备。
        if let Some(stored) = stored.filter(|_| self.storage.has_store()) {
            return MatrixSession::restore(&config(stored.homeserver_url), stored.session)
                .await
                .map_err(|e| (failure(&e), format!("恢复 Matrix 会话失败：{e}")));
        }
        let core_url = self.home.core_url().ok_or((
            ChatFailure::NotReady,
            "还没有家里服务器的连接信息".to_owned(),
        ))?;
        let token = login::login_token(&core_url, proxy, login::LOGIN_TOKEN_TIMEOUT).await?;
        // 没有登录状态时，本地存储里的旧设备密钥属于别的设备，新登录用不上。
        self.storage.forget_session().map_err(storage_failure)?;
        let session =
            MatrixSession::login_with_jwt(&config(token.homeserver_url.clone()), &token.token)
                .await
                .map_err(|e| (failure(&e), format!("登录 Matrix 失败：{e}")))?;
        if session.user_id().as_str() != token.user_id {
            tracing::warn!(expected = %token.user_id, actual = %session.user_id(), "登录得到的用户和核心说的不一样");
        }
        self.save(&session, token.homeserver_url, server_id)
            .map_err(storage_failure)?;
        Ok(session)
    }

    /// 存下的登录状态，只要 `server_id` 这台服务器的。别的服务器的令牌拿到这里只会被拒，看着像这台设备被吊销了，那边的
    /// 设备密钥和房间密钥也用不上，所以连本地存储一起丢掉，页面上也不再留那台服务器的房间。
    fn session_on(&self, server_id: &str) -> Result<Option<StoredSession>, CoreError> {
        let Some(stored) = self.storage.session()? else {
            return Ok(None);
        };
        if stored.server_id == server_id {
            return Ok(Some(stored));
        }
        tracing::info!(
            server = server_id,
            "换了服务器，丢掉旧的 Matrix 会话和本地存储"
        );
        self.storage.forget_session()?;
        self.view.send_modify(|view| {
            view.rooms.clear();
            view.problem = None;
            view.encryption.verification = DeviceVerification::Unknown;
            view.encryption.recovery = RecoveryStatus::Unknown;
            super::recovery::update_reminder(&mut view.encryption);
        });
        Ok(None)
    }

    pub(super) fn save(
        &self,
        session: &MatrixSession,
        homeserver_url: String,
        server_id: String,
    ) -> Result<(), CoreError> {
        let Some(saved) = session.saved() else {
            return Err(CoreError::Storage {
                reason: "登录后取不到登录状态".to_owned(),
            });
        };
        self.storage.save_session(&StoredSession {
            server_id,
            homeserver_url,
            session: saved,
        })
    }
}
