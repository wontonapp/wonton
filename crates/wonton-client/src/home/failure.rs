//! 连接失败的可读说明与原始诊断分开；不从底层错误字符串猜类别或超时。

use std::time::Duration;

use wonton_l10n::{Lang, l10n};
use wonton_protocol::{PairingFailure, PairingStatus};
use wonton_tunnel::TunnelError;
use wonton_tunnel::pair::PairingCode;
use wonton_tunnel::relays::Relay;

use crate::l10n::CATALOG;

pub(super) fn seconds(duration: Duration) -> f64 {
    let milliseconds = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    milliseconds as f64 / 1_000.0
}

pub(super) fn summary(error: &TunnelError, lang: Lang) -> String {
    let id = match error {
        TunnelError::Unauthorized => "home-failure-unauthorized",
        TunnelError::UpstreamUnavailable => "home-failure-upstream",
        TunnelError::Protocol { .. } => "home-failure-protocol",
        TunnelError::Closed => "home-tunnel-closed",
        TunnelError::TransportUnavailable { .. } => "home-failure-transport",
        _ => "home-failure-transport",
    };
    l10n!(CATALOG, lang, id)
}

pub(super) fn diagnostic(error: &TunnelError, lang: Lang) -> String {
    l10n!(
        CATALOG,
        lang,
        "home-tunnel-diagnostic",
        code = error.code(),
        error = error.to_string()
    )
}

pub(super) fn redact_relays(relays: &[Relay], text: &str) -> String {
    let mut safe = text.to_owned();
    for relay in relays {
        let url = relay.url.as_str();
        let address = url.split(['?', '#']).next().unwrap_or_default();
        safe = safe.replace(url, address);
        if let Some(token) = &relay.token
            && !token.expose().is_empty()
        {
            let token = token.expose();
            let quoted = serde_json::to_string(token).expect("relay token serializes to JSON");
            let json = &quoted[1..quoted.len() - 1];
            let encoded: String = url::form_urlencoded::byte_serialize(token.as_bytes()).collect();
            let json_encoded: String =
                url::form_urlencoded::byte_serialize(json.as_bytes()).collect();
            for secret in [token, json, encoded.as_str(), json_encoded.as_str()] {
                safe = safe.replace(secret, "[redacted]");
                safe = safe.replace(&secret.replace('+', "%20"), "[redacted]");
            }
        }
    }
    safe
}

pub(super) fn redact(code: &PairingCode, text: &str) -> String {
    redact_relays(&code.relays, text).replace(code.secret.expose(), "[redacted]")
}

pub(super) fn pairing(
    code: &PairingCode,
    lang: Lang,
    elapsed: Duration,
    limit: Duration,
    error: &TunnelError,
    bind_failed: bool,
) -> PairingStatus {
    let reason = match error {
        TunnelError::Protocol { .. } | TunnelError::Unauthorized => PairingFailure::Protocol,
        _ => PairingFailure::TransportUnavailable,
    };
    let relays = code
        .relays
        .iter()
        .map(|relay| {
            relay
                .url
                .as_str()
                .split(['?', '#'])
                .next()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(", ");
    PairingStatus::Failed {
        reason,
        message: l10n!(
            CATALOG,
            lang,
            "home-pairing-failed-detail",
            reason = if bind_failed {
                l10n!(CATALOG, lang, "home-pairing-network-setup-failed")
            } else {
                summary(error, lang)
            },
            elapsed_seconds = seconds(elapsed),
            stage_seconds = seconds(limit)
        ),
        diagnostic: Some(l10n!(
            CATALOG,
            lang,
            "home-pairing-diagnostic",
            addresses = format!("{:?}", code.direct_addrs),
            relays = redact(code, &relays),
            error = redact(code, &diagnostic(error, lang))
        )),
    }
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod failure_tests;
