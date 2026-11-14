//! 连接失败的可读说明与原始诊断分开；不从底层错误字符串猜类别或超时。

use std::{borrow::Cow, ops::Range, time::Duration};

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

const REDACTED: &str = "[redacted]";
const DECODE_WORK_FACTOR: usize = 16;

#[derive(Clone, Copy)]
struct SourceByte {
    byte: u8,
    start: usize,
    end: usize,
}

fn add_pattern<'a>(patterns: &mut Vec<Cow<'a, str>>, value: Cow<'a, str>) {
    if value.is_empty() {
        return;
    }
    if value
        .bytes()
        .any(|byte| byte < 0x20 || byte == b'"' || byte == b'\\')
    {
        let json = serde_json::to_string(value.as_ref()).expect("secret serializes to JSON");
        patterns.push(Cow::Owned(json[1..json.len() - 1].to_owned()));
    }
    patterns.push(value);
}

fn sensitive_patterns<'a>(relays: &'a [Relay], secret: Option<&'a str>) -> Vec<Cow<'a, str>> {
    let mut patterns = Vec::new();
    if let Some(secret) = secret {
        add_pattern(&mut patterns, Cow::Borrowed(secret));
    }
    for relay in relays {
        let url = relay.url.as_str();
        let address = url.split(['?', '#']).next().unwrap_or_default();
        let suffix = &url[address.len()..];
        if suffix.len() > 1 {
            add_pattern(&mut patterns, Cow::Borrowed(suffix));
        }
        if let Some(query) = relay.url.query() {
            add_pattern(&mut patterns, Cow::Borrowed(query));
            for part in query.split('&') {
                if let Some((_, value)) = part.split_once('=') {
                    add_pattern(&mut patterns, Cow::Borrowed(value));
                }
            }
            for (_, value) in relay.url.query_pairs() {
                add_pattern(&mut patterns, value);
            }
        }
        if let Some(fragment) = relay.url.fragment() {
            add_pattern(&mut patterns, Cow::Borrowed(fragment));
        }
        if let Some(token) = &relay.token {
            add_pattern(&mut patterns, Cow::Borrowed(token.expose()));
        }
    }
    patterns
}

fn byte_matches(actual: u8, expected: u8) -> bool {
    actual == expected || (actual == b'+' && expected == b' ')
}

fn mark_plain(text: &[u8], patterns: &[Cow<'_, str>], spans: &mut Vec<Range<usize>>) {
    for pattern in patterns {
        let pattern = pattern.as_bytes();
        for (start, candidate) in text.windows(pattern.len()).enumerate() {
            if candidate
                .iter()
                .zip(pattern)
                .all(|(&a, &b)| byte_matches(a, b))
            {
                spans.push(start..start + pattern.len());
            }
        }
    }
}

fn mark_decoded(bytes: &[SourceByte], patterns: &[Cow<'_, str>], spans: &mut Vec<Range<usize>>) {
    for pattern in patterns {
        let pattern = pattern.as_bytes();
        for candidate in bytes.windows(pattern.len()) {
            if candidate
                .iter()
                .zip(pattern)
                .all(|(a, &b)| byte_matches(a.byte, b))
            {
                spans.push(candidate[0].start..candidate[candidate.len() - 1].end);
            }
        }
    }
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_layer(bytes: &[SourceByte]) -> Option<Vec<SourceByte>> {
    let first = bytes.windows(3).position(|part| {
        part[0].byte == b'%' && hex(part[1].byte).is_some() && hex(part[2].byte).is_some()
    })?;
    let mut decoded = Vec::with_capacity(bytes.len() - 2);
    decoded.extend_from_slice(&bytes[..first]);
    let mut i = first;
    while i < bytes.len() {
        if let [first, high, low, ..] = &bytes[i..]
            && first.byte == b'%'
            && let Some(value) = hex(high.byte).zip(hex(low.byte)).map(|(h, l)| h * 16 + l)
        {
            decoded.push(SourceByte {
                byte: value,
                start: first.start,
                end: low.end,
            });
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    Some(decoded)
}

fn redact_patterns(text: &str, patterns: &[Cow<'_, str>]) -> String {
    if patterns.is_empty() {
        return text.to_owned();
    }
    let mut spans = Vec::new();
    mark_plain(text.as_bytes(), patterns, &mut spans);
    if text
        .as_bytes()
        .windows(3)
        .any(|part| part[0] == b'%' && hex(part[1]).is_some() && hex(part[2]).is_some())
    {
        let mut bytes: Vec<_> = text
            .bytes()
            .enumerate()
            .map(|(i, byte)| SourceByte {
                byte,
                start: i,
                end: i + 1,
            })
            .collect();
        let mut work = 0usize;
        while let Some(decoded) = decode_layer(&bytes) {
            work = work.saturating_add(bytes.len());
            if work > text.len().saturating_mul(DECODE_WORK_FACTOR) {
                return "[redacted: excessive percent encoding]".to_owned();
            }
            mark_decoded(&decoded, patterns, &mut spans);
            bytes = decoded;
        }
    }
    replace_spans(text, spans)
}

fn replace_spans(text: &str, mut spans: Vec<Range<usize>>) -> String {
    if spans.is_empty() {
        return text.to_owned();
    }
    for span in &mut spans {
        while !text.is_char_boundary(span.start) {
            span.start -= 1;
        }
        while !text.is_char_boundary(span.end) {
            span.end += 1;
        }
    }
    spans.sort_unstable_by_key(|span| span.start);
    let mut result = String::with_capacity(text.len());
    let mut previous = 0;
    let mut iter = spans.into_iter().peekable();
    while let Some(mut span) = iter.next() {
        while iter.peek().is_some_and(|next| next.start <= span.end) {
            span.end = span.end.max(iter.next().expect("peeked span").end);
        }
        result.push_str(&text[previous..span.start]);
        result.push_str(REDACTED);
        previous = span.end;
    }
    result.push_str(&text[previous..]);
    result
}

pub(super) fn redact_relays(relays: &[Relay], text: &str) -> String {
    redact_patterns(text, &sensitive_patterns(relays, None))
}

pub(super) fn redact(code: &PairingCode, text: &str) -> String {
    redact_patterns(
        text,
        &sensitive_patterns(&code.relays, Some(code.secret.expose())),
    )
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
