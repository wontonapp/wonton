//! 出码：一张长期有效的请求令牌，只在主人重置时换。

pub(crate) use wonton_tunnel::peer_pair::PeerPairToken;

use wonton_protocol::{PeerCode, SecretString};
use wonton_tunnel::peer_pair::{PeerPairingCode, PeerRejectReason, TOKEN_LEN};

use super::Peers;

/// 发一个新令牌。
pub(crate) fn issue() -> PeerPairToken {
    let mut bytes = [0u8; TOKEN_LEN];
    getrandom::fill(&mut bytes).expect("系统随机数不可用");
    PeerPairToken::from_bytes(bytes)
}

/// 验令牌：不是本机这一张就说明对方拿的是重置前的码。
pub(crate) fn verify(issued: &Option<PeerPairToken>, token: &PeerPairToken) -> Result<(), Reject> {
    match issued {
        Some(issued) if issued == token => Ok(()),
        Some(_) => Err(Reject {
            reason: PeerRejectReason::CodeRevoked,
            message: "这张码已经作废，请对方重新发一张".to_owned(),
        }),
        None => Err(Reject {
            reason: PeerRejectReason::CodeRevoked,
            message: "服务器没有发过配对码".to_owned(),
        }),
    }
}

/// 出码方拒绝一次扫码的原因，照 [`PeerRejectReason`] 回给扫码方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reject {
    pub(crate) reason: PeerRejectReason,
    pub(crate) message: String,
}

impl Peers {
    /// 本机的码：令牌一直是这一张，中继地址按当时的取。
    pub fn issue_code(&self) -> PeerCode {
        let inner = self.lock();
        let token = match inner.token.clone() {
            Some(token) => token,
            None => {
                let token = issue();
                let mut inner = inner;
                inner.token = Some(token.clone());
                if let Err(e) = self.save(&inner) {
                    tracing::error!(error = %e, "新配的码写不进去");
                }
                token
            }
        };
        self.code_uri(&token)
    }

    /// 重置码：换一个新令牌，旧码作废，用旧码发来的待处理请求一并清掉。
    pub fn reset_code(&self) -> PeerCode {
        let token = issue();
        {
            let mut inner = self.lock();
            inner.token = Some(token.clone());
            inner.requests.clear();
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "重置的码写不进去");
            }
        }
        tracing::info!("主人重置了服务器配对码");
        self.code_uri(&token)
    }

    fn code_uri(&self, token: &PeerPairToken) -> PeerCode {
        PeerCode {
            code: SecretString::new(
                PeerPairingCode {
                    server_id: self.tunnel.endpoint.id(),
                    relay_urls: self
                        .tunnel
                        .relays()
                        .iter()
                        .map(|relay| relay.url.clone())
                        .collect(),
                    server_name: self.own.server_name.clone(),
                    token: token.clone(),
                }
                .to_uri(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_token_verifies_and_an_older_one_is_revoked() {
        let issued = issue();
        assert!(verify(&Some(issued.clone()), &issued).is_ok());
        assert_eq!(
            verify(&Some(issued.clone()), &issue()).unwrap_err().reason,
            PeerRejectReason::CodeRevoked
        );
        assert_eq!(
            verify(&None, &issued).unwrap_err().reason,
            PeerRejectReason::CodeRevoked
        );
    }
}
