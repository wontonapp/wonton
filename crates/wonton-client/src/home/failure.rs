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

fn summary_key(error: &TunnelError) -> &'static str {
    match error {
        TunnelError::Unauthorized => "home-failure-unauthorized",
        TunnelError::UpstreamUnavailable => "home-failure-upstream",
        TunnelError::Protocol { .. } => "home-failure-protocol",
        TunnelError::Closed => "home-tunnel-closed",
        TunnelError::TransportUnavailable { .. } => "home-failure-transport",
        _ => "home-failure-transport",
    }
}

pub(super) fn summary(error: &TunnelError, lang: Lang) -> String {
    l10n!(CATALOG, lang, summary_key(error))
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
            for secret in [token, json] {
                if let Some(redacted) = redact_encoded(&safe, secret) {
                    safe = redacted;
                }
            }
        }
    }
    safe
}

fn redact_encoded(text: &str, token: &str) -> Option<String> {
    if !text.contains('%') && !(token.contains(' ') && text.contains('+')) {
        return None;
    }
    let mut result: Option<String> = None;
    let mut position = 0;
    let mut copied = 0;
    while position < text.len() {
        let length = encoded_prefix(&text.as_bytes()[position..], token.as_bytes());
        if let Some(end) = length.map(|length| position + length)
            && text.is_char_boundary(end)
        {
            let output = result.get_or_insert_with(|| String::with_capacity(text.len()));
            output.push_str(&text[copied..position]);
            output.push_str("[redacted]");
            copied = end;
            position = end;
        } else {
            position += text[position..].chars().next()?.len_utf8();
        }
    }
    if let Some(output) = &mut result {
        output.push_str(&text[copied..]);
    }
    result
}

fn encoded_prefix(text: &[u8], token: &[u8]) -> Option<usize> {
    let mut position = 0;
    for &expected in token {
        let first = *text.get(position)?;
        let escaped = if first == b'%' {
            let high = text
                .get(position + 1)
                .and_then(|byte| char::from(*byte).to_digit(16));
            let low = text
                .get(position + 2)
                .and_then(|byte| char::from(*byte).to_digit(16));
            high.zip(low).map(|(high, low)| (high * 16 + low) as u8)
        } else {
            None
        };
        if escaped == Some(expected) {
            position += 3;
        } else if first == expected || (first == b'+' && expected == b' ') {
            position += 1;
        } else {
            return None;
        }
    }
    Some(position)
}

pub(super) fn redact(code: &PairingCode, text: &str) -> String {
    let secret = code.secret.expose();
    let safe = redact_relays(&code.relays, text).replace(secret, "[redacted]");
    redact_encoded(&safe, secret).unwrap_or(safe)
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
        message: pairing_message(pairing_reason_key(error, bind_failed), elapsed, limit, lang),
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

pub(super) fn pairing_reason_key(error: &TunnelError, bind_failed: bool) -> &'static str {
    if bind_failed {
        "home-pairing-network-setup-failed"
    } else {
        summary_key(error)
    }
}

pub(super) fn pairing_message(
    reason: &'static str,
    elapsed: Duration,
    limit: Duration,
    lang: Lang,
) -> String {
    l10n!(
        CATALOG,
        lang,
        "home-pairing-failed-detail",
        reason = l10n!(CATALOG, lang, reason),
        elapsed_seconds = seconds(elapsed),
        stage_seconds = seconds(limit)
    )
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod failure_tests;
