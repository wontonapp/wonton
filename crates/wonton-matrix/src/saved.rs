//! 存下来的登录状态。两种认证 API 各存各的：密码和 JWT 走 Matrix 原生登录，只有用户、设备和令牌；
//! OAuth 2.0（ADR-0051 的 OIDC 登录）还要存注册得到的 `client_id`，刷新令牌时要用。

use matrix_sdk::{
    AuthSession, Client, SessionMeta, SessionTokens,
    authentication::matrix::MatrixSession as SdkSession,
    authentication::oauth::{ClientId, OAuthSession, UserSession},
    ruma::{DeviceId, UserId},
};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// 存下来的登录状态：用户、设备和令牌。重启后交给 [`MatrixSession::restore`](crate::MatrixSession::restore)，
/// 不用重新登录。
///
/// 令牌等同于登录凭据，要和设备密钥一样保管；`Debug` 不输出令牌。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SavedSession(Saved);

/// 不加标签：原生登录的写法和以前一样（字段平铺），OAuth 多一个 `client_id`。OAuth 必须排在前面：
/// 原生登录的写法会忽略多出来的 `client_id`，排在前面就把 OAuth 的会话读成原生的了。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum Saved {
    OAuth {
        client_id: String,
        #[serde(flatten)]
        meta: SessionMeta,
        #[serde(flatten)]
        tokens: SessionTokens,
    },
    Matrix(SdkSession),
}

impl SavedSession {
    /// 这份登录状态属于的用户。
    pub fn user_id(&self) -> &UserId {
        &self.meta().user_id
    }

    /// 这份登录状态属于的设备。
    pub fn device_id(&self) -> &DeviceId {
        &self.meta().device_id
    }

    fn meta(&self) -> &SessionMeta {
        match &self.0 {
            Saved::OAuth { meta, .. } => meta,
            Saved::Matrix(session) => &session.meta,
        }
    }

    /// 客户端当前的登录状态。
    pub(crate) fn of(client: &Client) -> Option<Self> {
        match client.session()? {
            AuthSession::Matrix(session) => Some(Self(Saved::Matrix(session))),
            AuthSession::OAuth(session) => Some(Self(Saved::OAuth {
                client_id: session.client_id.as_str().to_owned(),
                meta: session.user.meta,
                tokens: session.user.tokens,
            })),
            _ => {
                tracing::warn!("unknown Matrix authentication API, not saving it");
                None
            }
        }
    }

    pub(crate) async fn restore_into(self, client: &Client) -> Result<()> {
        match self.0 {
            Saved::Matrix(session) => client.restore_session(session).await?,
            Saved::OAuth {
                client_id,
                meta,
                tokens,
            } => client
                .restore_session(OAuthSession {
                    client_id: ClientId::new(client_id),
                    user: UserSession { meta, tokens },
                })
                .await
                .map_err(|error| Error::Upstream(format!("恢复 OAuth 会话失败：{error}")))?,
        }
        Ok(())
    }
}

impl std::fmt::Debug for SavedSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let api = match &self.0 {
            Saved::OAuth { .. } => "oauth",
            Saved::Matrix(_) => "matrix",
        };
        f.debug_struct("SavedSession")
            .field("api", &api)
            .field("user_id", &self.user_id())
            .field("device_id", &self.device_id())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> SessionTokens {
        SessionTokens {
            access_token: "private-access".to_owned(),
            refresh_token: Some("private-refresh".to_owned()),
        }
    }

    fn meta() -> SessionMeta {
        SessionMeta {
            user_id: "@alice:example.org".try_into().unwrap(),
            device_id: "PHONE".into(),
        }
    }

    /// 原生登录存成平铺的字段（和以前一样），OAuth 多一个 client_id；两种都读得回来，不会互相读错。
    #[test]
    fn both_apis_round_trip_without_reading_one_as_the_other() {
        let native = SavedSession(Saved::Matrix(SdkSession {
            meta: meta(),
            tokens: tokens(),
        }));
        let oauth = SavedSession(Saved::OAuth {
            client_id: "client-1".to_owned(),
            meta: meta(),
            tokens: tokens(),
        });
        let native_json = serde_json::to_value(&native).unwrap();
        assert_eq!(native_json["access_token"], "private-access");
        assert!(native_json.get("client_id").is_none());
        for saved in [native, oauth] {
            let json = serde_json::to_string(&saved).unwrap();
            assert_eq!(serde_json::from_str::<SavedSession>(&json).unwrap(), saved);
        }
    }

    #[test]
    fn debug_hides_tokens() {
        let saved = SavedSession(Saved::OAuth {
            client_id: "client-1".to_owned(),
            meta: meta(),
            tokens: tokens(),
        });
        let printed = format!("{saved:?}");
        assert!(!printed.contains("private-"), "{printed}");
        assert!(printed.contains("@alice:example.org"), "{printed}");
    }
}
