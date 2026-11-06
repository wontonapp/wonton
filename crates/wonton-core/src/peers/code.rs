//! 出码：一次性密钥，同一时间只有一张，有效期内能用一次。

use std::time::Duration;

use wonton_tunnel::peer_pair::{PeerPairSecret, PeerRejectReason, SECRET_LEN};

/// 码的有效期（ADR-0050：10 分钟）。
pub(crate) const CODE_TTL: Duration = Duration::from_secs(600);

/// 主人当前出的码。
#[derive(Debug, Clone)]
pub(crate) struct IssuedCode {
    pub(crate) secret: PeerPairSecret,
    /// 过期时刻（Unix 毫秒）。
    pub(crate) expires_at_ms: u64,
}

/// 发一个新码。
pub(crate) fn issue(ttl: Duration) -> IssuedCode {
    let mut bytes = [0u8; SECRET_LEN];
    getrandom::fill(&mut bytes).expect("系统随机数不可用");
    IssuedCode {
        secret: PeerPairSecret::from_bytes(bytes),
        expires_at_ms: super::now_ms() + u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX),
    }
}

/// 验码：密钥和有效期。一次性由调用方在验过之后立刻作废。
pub(crate) fn verify(issued: &Option<IssuedCode>, secret: &PeerPairSecret) -> Result<(), Reject> {
    let Some(issued) = issued else {
        return Err(Reject {
            reason: PeerRejectReason::UnknownCode,
            message: "服务器没有发过配对码".to_owned(),
        });
    };
    if issued.secret != *secret {
        return Err(Reject {
            reason: PeerRejectReason::UnknownCode,
            message: "没发过这个配对码".to_owned(),
        });
    }
    if issued.expires_at_ms <= super::now_ms() {
        return Err(Reject {
            reason: PeerRejectReason::Expired,
            message: "配对码过期了，请换一个新码".to_owned(),
        });
    }
    Ok(())
}

/// 出码方拒绝一次扫码的原因，照 [`PeerRejectReason`] 回给扫码方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reject {
    pub(crate) reason: PeerRejectReason,
    pub(crate) message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_code_verifies_and_the_wrong_secret_does_not() {
        let issued = issue(Duration::from_secs(600));
        assert!(verify(&Some(issued.clone()), &issued.secret).is_ok());
        assert_eq!(
            verify(&Some(issued), &PeerPairSecret::from_bytes([2; SECRET_LEN]))
                .unwrap_err()
                .reason,
            PeerRejectReason::UnknownCode
        );
        assert_eq!(
            verify(&None, &PeerPairSecret::from_bytes([1; SECRET_LEN]))
                .unwrap_err()
                .reason,
            PeerRejectReason::UnknownCode
        );
    }

    #[test]
    fn an_expired_code_is_expired() {
        let mut issued = issue(Duration::from_secs(600));
        // 验码先看密钥再看有效期：给码一个已知的密钥，过期才能轮到。
        issued.secret = PeerPairSecret::from_bytes([1; SECRET_LEN]);
        issued.expires_at_ms = crate::peers::now_ms().saturating_sub(1);
        assert_eq!(
            verify(&Some(issued), &PeerPairSecret::from_bytes([1; SECRET_LEN]))
                .unwrap_err()
                .reason,
            PeerRejectReason::Expired
        );
    }
}
