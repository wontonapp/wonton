//! 配对码校验的可读原因，以及不含输入值的安全诊断。

use wonton_l10n::{Lang, l10n};
use wonton_tunnel::pair::{InvalidCode, RejectReason};

use crate::l10n::CATALOG;

pub(super) fn summary(error: &InvalidCode, lang: Lang) -> String {
    let key = match error {
        InvalidCode::NotPairingCode => "home-code-not-pairing",
        InvalidCode::UnsupportedVersion(_) => "home-code-unsupported-version",
        InvalidCode::Malformed { field, .. } => match *field {
            "version" => "home-code-invalid-version",
            "server ID" | "direct address" | "server address" => "home-code-invalid-address",
            "relay table" => "home-code-invalid-relay",
            "base domain" | "core URL" => "home-code-invalid-domain",
            "pairing secret" => "home-code-invalid-secret",
            _ => "home-code-invalid",
        },
    };
    l10n!(CATALOG, lang, key)
}

pub(super) fn rejection(reason: RejectReason, lang: Lang) -> String {
    let key = match reason {
        RejectReason::UnknownCode | RejectReason::Other => "home-code-rejected",
        RejectReason::Expired => "home-code-expired",
        RejectReason::Used => "home-code-used",
        RejectReason::Closed => "home-code-closed",
        RejectReason::BadRequest => "home-code-request-invalid",
        RejectReason::ServerError => "home-code-server-error",
    };
    l10n!(CATALOG, lang, key)
}
pub(super) fn diagnostic(error: &InvalidCode) -> String {
    match error {
        InvalidCode::NotPairingCode => error.to_string(),
        InvalidCode::UnsupportedVersion(_) => "unsupported pairing code version (redacted)".into(),
        InvalidCode::Malformed { field, reason } => {
            let safe = match reason.as_str() {
                "missing v"
                | "missing id"
                | "no relay URL or direct address"
                | "missing domain"
                | "missing core"
                | "expected 32 lowercase hex digits" => reason.as_str(),
                _ => "details redacted",
            };
            format!("invalid {field} in the pairing code: {safe}")
        }
    }
}

#[cfg(test)]
#[path = "pairing_code_tests.rs"]
mod pairing_code_tests;
