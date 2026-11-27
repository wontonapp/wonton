use super::*;
use iroh::SecretKey;
use std::time::Instant;
use wonton_tunnel::pair::{PairingSecret, SECRET_LEN};
use wonton_tunnel::relays::RelayView;

fn code() -> PairingCode {
    PairingCode {
        server_id: SecretKey::from_bytes(&[7; 32]).public(),
        direct_addrs: vec!["192.0.2.10:18482".parse().unwrap()],
        relays: vec![RelayView {
            url: "https://relay.example.com".parse().unwrap(),
            token: None,
            quic_port: None,
            tier: 0,
        }],
        base_domain: "wonton.localhost".into(),
        core_url: "http://wonton.localhost:18480/".parse().unwrap(),
        secret: PairingSecret::from_bytes([42; SECRET_LEN]),
    }
}

fn numbers(text: &str) -> Vec<f64> {
    text.split(|character: char| !character.is_ascii_digit() && character != '.')
        .filter_map(|number| number.parse().ok())
        .collect()
}

#[test]
fn pairing_details_keep_seconds_languages_and_raw_diagnostics_separate() {
    let code = code();
    let error = TunnelError::TransportUnavailable {
        reason: "pairing did not finish within 30s".into(),
    };
    let mut messages = Vec::new();
    for tag in ["en", "zh-Hans", "zh-Hant", "ja"] {
        let lang = CATALOG.negotiate(&[tag]);
        let PairingStatus::Failed {
            reason,
            message,
            diagnostic,
        } = pairing(
            &code,
            lang,
            Duration::new(72, 208_467_000),
            Duration::from_secs(30),
            &error,
            false,
        )
        else {
            panic!("连接失败不能显示为成功");
        };
        assert_eq!(reason, PairingFailure::TransportUnavailable);
        assert_eq!(numbers(&message), [72.208, 30.0], "{tag}: {message}");
        assert!(
            !message.contains("tunnel transport unavailable"),
            "{message}"
        );
        assert!(!message.contains("192.0.2.10"), "{message}");
        assert!(!message.contains("relay.example.com"), "{message}");
        if tag == "en" {
            assert!(message.is_ascii(), "{message}");
        } else if tag == "ja" {
            assert!(
                message
                    .chars()
                    .any(|c| ('\u{3040}'..='\u{30ff}').contains(&c)),
                "{message}"
            );
        } else {
            assert!(
                message
                    .chars()
                    .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                "{message}"
            );
        }
        let diagnostic = diagnostic.expect("诊断必须保留");
        for raw in [
            error.code(),
            &error.to_string(),
            "192.0.2.10:18482",
            "https://relay.example.com",
        ] {
            assert!(diagnostic.contains(raw), "{tag}: {diagnostic}");
        }
        assert!(!diagnostic.contains(code.secret.expose()));
        assert!(
            messages.iter().all(|previous| previous != &message),
            "{tag}: {message}"
        );
        messages.push(message);
    }
}

#[test]
fn subsecond_details_do_not_truncate_elapsed_or_stage_limits() {
    let error = TunnelError::TransportUnavailable {
        reason: "network setup failed".into(),
    };
    let PairingStatus::Failed { message, .. } = pairing(
        &code(),
        CATALOG.negotiate(&["en"]),
        Duration::from_millis(125),
        Duration::from_millis(300),
        &error,
        false,
    ) else {
        unreachable!()
    };
    assert_eq!(numbers(&message), [0.125, 0.3], "{message}");
}

#[test]
fn pairing_diagnostics_remove_secrets_and_relay_tokens() {
    let mut code = code();
    let token = "relay-token private&=\"";
    code.relays[0].token = Some(wonton_tunnel::relays::RelayToken::new(token));
    let uri = code.to_uri();
    let raw = format!("{} {} {}", uri, code.secret.expose(), token);
    let error = TunnelError::TransportUnavailable {
        reason: raw.clone(),
    };
    let PairingStatus::Failed {
        message,
        diagnostic: Some(diagnostic),
        ..
    } = pairing(
        &code,
        CATALOG.negotiate(&["en"]),
        Duration::from_secs(1),
        Duration::from_secs(30),
        &error,
        false,
    )
    else {
        panic!("连接失败应保留诊断");
    };
    let encoded: String = url::form_urlencoded::byte_serialize(token.as_bytes()).collect();
    for value in [
        raw.as_str(),
        uri.as_str(),
        code.secret.expose(),
        token,
        &encoded,
    ] {
        assert!(!message.contains(value));
        assert!(!diagnostic.contains(value));
    }
    assert!(diagnostic.contains("192.0.2.10:18482"));
    assert!(diagnostic.contains("relay.example.com"));
}

#[test]
fn deeply_encoded_pairing_error_is_explicitly_redacted() {
    let encoded = format!("%{}2b", "25".repeat(100));
    let error = TunnelError::TransportUnavailable {
        reason: format!("peer closed: {encoded}"),
    };
    let PairingStatus::Failed {
        reason,
        diagnostic: Some(diagnostic),
        ..
    } = pairing(
        &code(),
        CATALOG.negotiate(&["en"]),
        Duration::from_secs(1),
        Duration::from_secs(30),
        &error,
        false,
    )
    else {
        panic!("connection failure must retain a diagnostic");
    };
    assert_eq!(reason, PairingFailure::TransportUnavailable);
    assert!(diagnostic.contains("[redacted: excessive percent encoding]"));
    assert!(!diagnostic.contains(&encoded));
}

#[test]
fn pattern_length_limit_counts_before_expanding() {
    let fits = "\u{1F600}".repeat(MAX_PATTERN_CHARS);
    let mut patterns = Patterns::default();
    patterns.add(&fits, false);
    assert!(patterns.exhausted.is_none());
    let text = format!("before {fits} after");
    assert_eq!(redact_patterns(&text, &patterns), "before [redacted] after");

    let too_long = "\u{1F600}".repeat(MAX_PATTERN_CHARS + 1);
    let mut patterns = Patterns::default();
    patterns.add(&too_long, false);
    assert_eq!(patterns.exhausted, Some(Exhaustion::Scale));
    assert!(patterns.list.is_empty(), "超长取值不应先展开写法");
    assert_eq!(redact_patterns(&too_long, &patterns), CROWDED);
}

#[test]
fn overlong_values_are_counted_before_spellings_are_built() {
    let value = "\u{1F600}".repeat(MAX_PATTERN_CHARS + 1);
    let rounds = 30;
    let expanded = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(value.chars().map(spellings).collect::<Vec<_>>());
    }
    let expanded = expanded.elapsed();
    let counted = Instant::now();
    for _ in 0..rounds {
        let mut patterns = Patterns::default();
        patterns.add(std::hint::black_box(&value), false);
        assert!(patterns.list.is_empty());
    }
    let counted = counted.elapsed();
    assert!(
        counted * 4 < expanded,
        "超长取值应在展开前被拒绝：计数 {counted:?}，展开 {expanded:?}"
    );
}

#[test]
fn hex_case_variants_of_one_value_share_a_single_pattern() {
    let mut patterns = Patterns::default();
    add_decoded(&mut patterns, "a%2Db", true);
    add_decoded(&mut patterns, "a%2db", true);
    assert_eq!(patterns.list.len(), 1);
    assert_eq!(redact_patterns("a-b", &patterns), REDACTED);
}
