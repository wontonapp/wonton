//! 秘密：恢复密钥，以后还有令牌和凭据。`Debug` 一律打码，哪里把 ViewModel 或操作打进日志也不会泄露。

use serde::{Deserialize, Serialize};

/// 一段秘密文字。`Debug` 只显示 `<redacted>`；要用原文时调 [`SecretString::expose`]，用到的地方一目了然。
///
/// 跨 FFI 时是普通字符串（Swift 和 Kotlin 的 `String`），壳那边要自己避免把它写进日志。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// 原文。
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(feature = "uniffi")]
uniffi::custom_newtype!(SecretString, String);
