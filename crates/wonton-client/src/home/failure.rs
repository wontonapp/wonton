//! 连接失败的可读说明与原始诊断分开；不从底层错误字符串猜类别或超时。

use std::{collections::HashSet, ops::Range, time::Duration};

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
/// 配置端取值的解码层数上限：敏感值本身不再变化时提前收手。
const URL_DECODE_LAYERS: usize = 3;

/// 一个敏感值的匹配视图：逐字符列出它可能被写成的字节写法（原文、`%XX`/JSON 层的解码目标、JSON 转义）。
type Pattern = Vec<Vec<Vec<u8>>>;

#[derive(Clone, Copy)]
struct SourceByte {
    byte: u8,
    start: usize,
    end: usize,
}

fn add_pattern(patterns: &mut Vec<Pattern>, value: &str) {
    if value.is_empty() {
        return;
    }
    patterns.push(value.chars().map(spellings).collect());
}

fn hex_case_variants(upper: &str) -> Vec<Vec<u8>> {
    let mut variants = vec![Vec::new()];
    for byte in upper.bytes() {
        let mut next = Vec::with_capacity(variants.len() * 2);
        for variant in &variants {
            let mut same = variant.clone();
            same.push(byte);
            next.push(same);
            if byte.is_ascii_uppercase() && byte <= b'F' {
                let mut lower = variant.clone();
                lower.push(byte.to_ascii_lowercase());
                next.push(lower);
            }
        }
        variants = next;
    }
    variants
}

fn spellings(character: char) -> Vec<Vec<u8>> {
    let mut buffer = [0u8; 4];
    let mut spellings = vec![character.encode_utf8(&mut buffer).as_bytes().to_vec()];
    let short = match character {
        '"' => Some("\\\""),
        '\\' => Some("\\\\"),
        '/' => Some("\\/"),
        '\u{8}' => Some("\\b"),
        '\u{c}' => Some("\\f"),
        '\n' => Some("\\n"),
        '\r' => Some("\\r"),
        '\t' => Some("\\t"),
        _ => None,
    };
    if let Some(short) = short {
        spellings.push(short.as_bytes().to_vec());
    }
    // 任意字符都可能被写成 `\uXXXX`；星平面字符是两个码元的代理对。
    for unit in character.encode_utf16(&mut [0u16; 2]) {
        spellings.push(format!("\\u{unit:04X}").into_bytes());
        for variant in hex_case_variants(&format!("\\u{unit:04X}")) {
            spellings.push(variant);
        }
    }
    spellings
}

/// 把配置端取值按它所在 URL 分量的规则再解几层，正文字面回显这些解码值时也能命中。
fn add_decoded(patterns: &mut Vec<Pattern>, value: &str) {
    let mut current = value.to_owned();
    for _ in 0..URL_DECODE_LAYERS {
        let Ok(decoded) = percent_encoding::percent_decode_str(&current).decode_utf8() else {
            return;
        };
        if decoded == current {
            return;
        }
        add_pattern(patterns, &decoded);
        current = decoded.into_owned();
    }
}

fn sensitive_patterns(relays: &[Relay], secret: Option<&str>) -> Vec<Pattern> {
    let mut patterns = Vec::new();
    if let Some(secret) = secret {
        add_pattern(&mut patterns, secret);
    }
    for relay in relays {
        let url = relay.url.as_str();
        let address = url.split(['?', '#']).next().unwrap_or_default();
        let suffix = &url[address.len()..];
        if suffix.len() > 1 {
            add_pattern(&mut patterns, suffix);
        }
        if let Some(query) = relay.url.query() {
            add_pattern(&mut patterns, query);
            for part in query.split('&') {
                if let Some((_, value)) = part.split_once('=') {
                    add_pattern(&mut patterns, value);
                    // 查询分量按表单规则解码：`+` 是空格。
                    add_decoded(&mut patterns, &value.replace('+', " "));
                }
            }
            for (_, value) in relay.url.query_pairs() {
                add_pattern(&mut patterns, &value);
                add_decoded(&mut patterns, &value);
            }
        }
        if let Some(fragment) = relay.url.fragment() {
            add_pattern(&mut patterns, fragment);
            // 片段按片段规则解码：`+` 不是空格。
            add_decoded(&mut patterns, fragment);
        }
        if let Some(token) = &relay.token {
            add_pattern(&mut patterns, token.expose());
        }
    }
    patterns
}

fn byte_matches(actual: u8, expected: u8) -> bool {
    actual == expected || (actual == b'+' && expected == b' ')
}

/// 从 `start` 起让每个字符命中它的一种写法；成功时返回原文里最后一个命中字节之后的偏移。
fn match_at(view: &[SourceByte], start: usize, pattern: &Pattern) -> Option<usize> {
    let mut stack = vec![(start, 0usize)];
    let mut seen = HashSet::new();
    while let Some((at, step)) = stack.pop() {
        if step == pattern.len() {
            return Some(if at == start {
                view[start].end
            } else {
                view[at - 1].end
            });
        }
        if !seen.insert((at, step)) {
            continue;
        }
        for spelling in &pattern[step] {
            let end = at + spelling.len();
            if end <= view.len()
                && view[at..end]
                    .iter()
                    .zip(spelling)
                    .all(|(actual, &expected)| byte_matches(actual.byte, expected))
            {
                stack.push((end, step + 1));
            }
        }
    }
    None
}

fn mark(view: &[SourceByte], patterns: &[Pattern], spans: &mut Vec<Range<usize>>) {
    for pattern in patterns {
        for start in 0..view.len() {
            if let Some(end) = match_at(view, start, pattern) {
                spans.push(view[start].start..end);
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

fn decode_layer(bytes: &[SourceByte]) -> Vec<SourceByte> {
    let Some(first) = bytes.windows(3).position(|part| {
        part[0].byte == b'%' && hex(part[1].byte).is_some() && hex(part[2].byte).is_some()
    }) else {
        return bytes.to_vec();
    };
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
    decoded
}

/// 解一层 JSON 字符串转义；转义写法本身只算一个可见字符的跨度。
fn json_layer(bytes: &[SourceByte]) -> Option<Vec<SourceByte>> {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut found = false;
    while i < bytes.len() {
        let escape = if bytes[i].byte == b'\\' {
            bytes.get(i + 1).map(|next| next.byte)
        } else {
            None
        };
        let replacement = match escape {
            Some(b'u') => unicode_escape(&bytes[i..]),
            Some(byte) => match byte {
                b'/' => Some((b"/".to_vec(), 2)),
                b'"' => Some((b"\"".to_vec(), 2)),
                b'\\' => Some((b"\\".to_vec(), 2)),
                b'b' => Some((b"\x08".to_vec(), 2)),
                b'f' => Some((b"\x0c".to_vec(), 2)),
                b'n' => Some((b"\n".to_vec(), 2)),
                b'r' => Some((b"\r".to_vec(), 2)),
                b't' => Some((b"\t".to_vec(), 2)),
                _ => None,
            },
            None => None,
        };
        let Some((text, consumed)) = replacement else {
            decoded.push(bytes[i]);
            i += 1;
            continue;
        };
        let start = bytes[i].start;
        let end = bytes[i + consumed - 1].end;
        decoded.extend(text.into_iter().map(|byte| SourceByte { byte, start, end }));
        i += consumed;
        found = true;
    }
    found.then_some(decoded)
}

fn unicode_escape(bytes: &[SourceByte]) -> Option<(Vec<u8>, usize)> {
    let unit = |at: usize| {
        if bytes.get(at)?.byte != b'u' {
            return None;
        }
        let mut value = 0u32;
        for offset in 1..5 {
            value = value * 16 + u32::from(hex(bytes.get(at + offset)?.byte)?);
        }
        Some(value)
    };
    let first = unit(1)?;
    let (character, consumed) = if (0xD800..0xDC00).contains(&first) {
        let second = unit(7)?;
        if !(0xDC00..0xE000).contains(&second) {
            return None;
        }
        (
            char::from_u32(0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00))?,
            12,
        )
    } else {
        (char::from_u32(first)?, 6)
    };
    let mut buffer = [0u8; 4];
    Some((
        character.encode_utf8(&mut buffer).as_bytes().to_vec(),
        consumed,
    ))
}

fn redact_patterns(text: &str, patterns: &[Pattern]) -> String {
    if patterns.is_empty() {
        return text.to_owned();
    }
    let mut spans = Vec::new();
    let mut bytes: Vec<_> = text
        .bytes()
        .enumerate()
        .map(|(i, byte)| SourceByte {
            byte,
            start: i,
            end: i + 1,
        })
        .collect();
    mark(&bytes, patterns, &mut spans);
    let mut work = 0usize;
    let mut decoded_once = false;
    // 百分号层与 JSON 层交替解析，两者叠加的写法也能命中；只看原文跨度，不先改写正文。
    loop {
        let percent = decode_layer(&bytes);
        let next = if percent.len() < bytes.len() {
            Some(percent)
        } else {
            json_layer(&bytes)
        };
        let Some(next) = next else {
            if decoded_once {
                break;
            }
            // 正文可能整段就是解码后的写法，没有转义也要按解码视图匹配一次。
            decoded_once = true;
            mark(&bytes, patterns, &mut spans);
            continue;
        };
        decoded_once = true;
        work = work.saturating_add(bytes.len());
        if work > text.len().saturating_mul(DECODE_WORK_FACTOR) {
            return "[redacted: excessive percent encoding]".to_owned();
        }
        mark(&next, patterns, &mut spans);
        bytes = next;
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
        diagnostic: Some(redact(
            code,
            &l10n!(
                CATALOG,
                lang,
                "home-pairing-diagnostic",
                addresses = format!("{:?}", code.direct_addrs),
                relays = relays,
                error = diagnostic(error, lang)
            ),
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
