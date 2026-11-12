//! 登录或恢复 Matrix 会话，以及存下的登录状态属于哪台服务器。

use wonton_l10n::l10n;
use wonton_matrix::{MatrixSession, SessionConfig, StoreConfig};
use wonton_protocol::{ChatFailure, CoreError, DeviceVerification, ProxySettings, RecoveryStatus};

use super::storage::StoredSession;
use super::{ChatLink, failure, login};
use crate::l10n::{CATALOG, language};

/// 连接信息里没有设备名时的显示名（`ConnectHome` 给的连接信息没有）。
const DEVICE_NAME: &str = "Wonton App";

impl ChatLink {
    pub(super) async fn sign_in(
        &self,
        proxy: &ProxySettings,
        server_id: String,
    ) -> Result<MatrixSession, (ChatFailure, String)> {
        let lang = language();
        // Matrix 设备名用配对时填的名字，主人才能在设备列表里认出是哪台。
        let device_name = self
            .home
            .device_name()
            .unwrap_or_else(|| DEVICE_NAME.to_owned());
        let config = |homeserver_url: String| SessionConfig {
            homeserver_url,
            device_name: device_name.clone(),
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
                .map_err(|e| {
                    (
                        failure(&e),
                        l10n!(
                            CATALOG,
                            lang,
                            "chat-sign-in-restore-failed",
                            reason = e.to_string()
                        ),
                    )
                });
        }
        let core_url = self.home.core_url().ok_or((
            ChatFailure::NotReady,
            l10n!(CATALOG, lang, "chat-sign-in-no-connection-info"),
        ))?;
        let token = login::login_token(&core_url, proxy, login::LOGIN_TOKEN_TIMEOUT).await?;
        // 没有登录状态时，本地存储里的旧设备密钥属于别的设备，新登录用不上。
        self.storage.forget_session().map_err(storage_failure)?;
        let session =
            MatrixSession::login_with_jwt(&config(token.homeserver_url.clone()), &token.token)
                .await
                .map_err(|e| {
                    (
                        failure(&e),
                        l10n!(
                            CATALOG,
                            lang,
                            "chat-sign-in-login-failed",
                            reason = e.to_string()
                        ),
                    )
                })?;
        if session.user_id().as_str() != token.user_id {
            tracing::warn!(
                expected = %token.user_id,
                actual = %session.user_id(),
                "signed-in user does not match the one the core reported"
            );
        }
        self.save(&session, token.homeserver_url, server_id)
            .map_err(storage_failure)?;
        // 扫码配对来的新设备接着等另一台设备交出密钥（ADR-0087）；之后恢复会话时靠这个标记接着交接。
        if self.home.paired_with_code() {
            self.storage.begin_key_handoff().map_err(storage_failure)?;
        }
        Ok(session)
    }

    /// 外部 Matrix 账号的槽位（ADR-0051）：只用登录时存下的状态和存储恢复，直连 homeserver，不经家里的代理。
    /// 没有登录状态（或存储丢了）就只能重新登录这个账号，按被拒处理，不自己去登录。
    pub(super) async fn sign_in_external(&self) -> Result<MatrixSession, (ChatFailure, String)> {
        let lang = language();
        let stored = self
            .storage
            .session()
            .map_err(|e| (ChatFailure::Upstream, e.to_string()))?
            .filter(|_| self.storage.has_store())
            .ok_or_else(|| {
                (
                    ChatFailure::Unauthorized,
                    l10n!(CATALOG, lang, "chat-sign-in-external-missing"),
                )
            })?;
        let config = SessionConfig {
            homeserver_url: stored.homeserver_url,
            device_name: DEVICE_NAME.to_owned(),
            store: StoreConfig::Sqlite {
                path: self.storage.store_dir(),
                passphrase: None,
            },
            proxy: None,
        };
        MatrixSession::restore(&config, stored.session)
            .await
            .map_err(|e| {
                (
                    failure(&e),
                    l10n!(
                        CATALOG,
                        lang,
                        "chat-sign-in-restore-failed",
                        reason = e.to_string()
                    ),
                )
            })
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
            "switched servers, dropping the old Matrix session and local store"
        );
        self.storage.forget_session()?;
        self.view.send_modify(|view| {
            view.rooms.clear();
            view.problem = None;
            view.encryption.verification = DeviceVerification::Unknown;
            view.encryption.recovery = RecoveryStatus::Unknown;
            super::recovery::refresh(&mut view.encryption);
        });
        Ok(None)
    }

    pub(super) fn save(
        &self,
        session: &MatrixSession,
        homeserver_url: String,
        server_id: String,
    ) -> Result<(), CoreError> {
        let lang = language();
        let Some(saved) = session.saved() else {
            return Err(CoreError::Storage {
                reason: l10n!(CATALOG, lang, "chat-sign-in-no-saved-session"),
            });
        };
        self.storage.save_session(&StoredSession {
            server_id,
            homeserver_url,
            session: saved,
        })
    }
}
