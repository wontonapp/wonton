//! 签发 Tuwunel 登录用的 JWT（ADR-0009 的验证结果 M0-2）：Ed25519 私钥只在核心，Tuwunel 只配公钥。

use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;

/// 和 Tuwunel 配置里 `[global.jwt]` 的 `issuer`、`audience` 一致。
pub const ISSUER: &str = "wonton-core";
pub const AUDIENCE: &str = "tuwunel";

/// 登录令牌只用来换一次访问令牌，越短越好；留出时钟偏差和弱网的余量。
pub const LOGIN_TOKEN_TTL_SECS: u64 = 120;

/// `sub` 是用户名（localpart），Tuwunel 拼上自己的 `server_name` 得到用户 ID。
#[derive(Serialize)]
struct Claims<'a> {
    sub: &'a str,
    iss: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
}

pub struct Issuer {
    key: EncodingKey,
}

impl Issuer {
    /// 读 PEM 格式的 Ed25519 私钥（`openssl genpkey -algorithm ed25519` 的输出）。
    pub fn from_pem(pem: &str) -> Result<Self, String> {
        EncodingKey::from_ed_pem(pem.as_bytes())
            .map(|key| Self { key })
            .map_err(|e| format!("Matrix 登录签发私钥不是 PEM 格式的 Ed25519 私钥：{e}"))
    }

    /// 给 `localpart` 签一个 [`LOGIN_TOKEN_TTL_SECS`] 秒内有效的登录令牌。
    pub fn login_token(&self, localpart: &str) -> String {
        let iat = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let claims = Claims {
            sub: localpart,
            iss: ISSUER,
            aud: AUDIENCE,
            iat,
            exp: iat + LOGIN_TOKEN_TTL_SECS,
        };
        jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), &claims, &self.key)
            .expect("Ed25519 签名不会失败")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use aws_lc_rs::encoding::AsDer;
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
    use jsonwebtoken::{DecodingKey, Validation};
    use serde_json::Value;

    use super::*;

    /// 测试用：一把新的签发密钥，和验签用的公钥。
    pub(crate) fn issuer_and_verifier() -> (Issuer, DecodingKey) {
        let pair = Ed25519KeyPair::generate().unwrap();
        let pkcs8 = pair.to_pkcs8().unwrap();
        let public = pair.public_key().as_der().unwrap();
        let issuer = Issuer {
            key: EncodingKey::from_ed_der(pkcs8.as_ref()),
        };
        (issuer, DecodingKey::from_ed_der(public.as_ref()))
    }

    /// 验签并取出声明，和 Tuwunel 一样要求签发方、接收方和 `exp`。
    pub(crate) fn verify(token: &str, key: &DecodingKey) -> Value {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        jsonwebtoken::decode::<Value>(token, key, &validation)
            .unwrap()
            .claims
    }

    #[test]
    fn login_tokens_carry_the_claims_tuwunel_checks() {
        let (issuer, key) = issuer_and_verifier();
        let claims = verify(&issuer.login_token("owner"), &key);
        assert_eq!(claims["sub"], "owner");
        let (iat, exp) = (
            claims["iat"].as_u64().unwrap(),
            claims["exp"].as_u64().unwrap(),
        );
        assert_eq!(exp - iat, LOGIN_TOKEN_TTL_SECS);
        // 别的密钥签的令牌验不过。
        let (other, _) = issuer_and_verifier();
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[AUDIENCE]);
        assert!(
            jsonwebtoken::decode::<Value>(&other.login_token("owner"), &key, &validation).is_err()
        );
        assert!(Issuer::from_pem("not a key").is_err());
    }
}
