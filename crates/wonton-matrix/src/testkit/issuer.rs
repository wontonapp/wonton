//! 模拟核心签发 Tuwunel 登录 JWT（ADR-0009）：Ed25519 私钥只在核心，Tuwunel 只配公钥。

use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::{
    encoding::AsDer,
    signature::{Ed25519KeyPair, KeyPair},
};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;

/// 与 deploy/dev/matrix/tuwunel.toml 的 `[global.jwt]` 一致。
pub const ISSUER: &str = "wonton-core";
pub const AUDIENCE: &str = "tuwunel";

/// 登录令牌的有效期。令牌只用来换一次访问令牌，越短越好；留出时钟偏差和弱网的余量。
pub const LOGIN_TOKEN_TTL_SECS: u64 = 120;

/// JWT 声明。`sub` 是用户名（localpart），Tuwunel 拼上自己的 server_name 得到 MXID。
#[derive(Clone, Debug, Serialize)]
pub struct Claims {
    pub sub: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    pub iat: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
}

impl Claims {
    /// 核心签发设备或 Bot 登录时用的声明。
    pub fn login(localpart: &str) -> Self {
        let now = now();
        Self {
            sub: localpart.to_owned(),
            iss: Some(ISSUER.to_owned()),
            aud: Some(AUDIENCE.to_owned()),
            iat: now,
            exp: Some(now + LOGIN_TOKEN_TTL_SECS),
        }
    }
}

/// 当前 Unix 时间（秒）。
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于 1970 年")
        .as_secs()
}

#[derive(Clone)]
pub struct Issuer {
    key: EncodingKey,
    public_key_pem: Option<String>,
}

impl Issuer {
    /// 生成一把新的签发密钥，公钥交给测试用的 Tuwunel。
    pub fn generate() -> Self {
        let pair = Ed25519KeyPair::generate().expect("生成 Ed25519 密钥");
        let pkcs8 = pair.to_pkcs8().expect("导出 PKCS#8 私钥");
        let spki = pair.public_key().as_der().expect("导出公钥");
        let public_key_pem = pem::encode(&pem::Pem::new("PUBLIC KEY", spki.as_ref()));
        Self {
            key: EncodingKey::from_ed_der(pkcs8.as_ref()),
            public_key_pem: Some(public_key_pem),
        }
    }

    /// 用现成服务器的私钥签发，例如 deploy/dev/matrix/dev.sh 生成的密钥。
    pub fn from_private_key_pem(pem: &[u8]) -> Self {
        let key = EncodingKey::from_ed_pem(pem).expect("读取 Ed25519 私钥（PEM）");
        Self {
            key,
            public_key_pem: None,
        }
    }

    /// 配给 Tuwunel 的公钥（PEM）。
    pub fn public_key_pem(&self) -> &str {
        self.public_key_pem
            .as_deref()
            .expect("只有测试生成的密钥才带公钥")
    }

    pub fn sign(&self, claims: &Claims) -> String {
        jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), claims, &self.key).expect("签发 JWT")
    }

    pub fn login_token(&self, localpart: &str) -> String {
        self.sign(&Claims::login(localpart))
    }
}
