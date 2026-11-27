use std::process::Command;

use iroh::SecretKey;
use url::Url;
use wonton_client::{CoreError, PairingFailure, PairingStatus};
use wonton_tunnel::pair::{PairingCode, PairingSecret, SECRET_LEN};
use wonton_tunnel::relays::{RelayToken, RelayView};

use super::support::{Home, Phone};

#[test]
fn invalid_pairing_input_and_server_rejection_keep_diagnostics_secret_safe() {
    const CHILD: &str = "WONTON_PAIR_REDACTION_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "pairing::invalid_details::invalid_pairing_input_and_server_rejection_keep_diagnostics_secret_safe",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated redaction test failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(assert_invalid_pairing_and_rejection());
}

async fn assert_invalid_pairing_and_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("pairing.log");
    wonton_client::init_logging(
        "wonton_client::home::pairing=info".to_owned(),
        Some(log_path.to_string_lossy().into_owned()),
    );
    let phone = Phone::open(dir.path());
    let code = PairingCode {
        server_id: SecretKey::generate().public(),
        relays: Vec::new(),
        direct_addrs: vec!["192.0.2.10:18482".parse().unwrap()],
        base_domain: "wonton.localhost".into(),
        core_url: Url::parse("http://wonton.localhost:18480/").unwrap(),
        secret: PairingSecret::from_bytes([42; SECRET_LEN]),
    };
    let uri = code.to_uri();
    let malformed = uri.replace("domain=wonton.localhost", "domain=invalid%2Fprivate-token");
    let Err(CoreError::InvalidInput { reason }) = phone.pair(&malformed) else {
        panic!("无效配对码必须当场拒绝");
    };
    for value in [uri.as_str(), code.secret.expose(), "private-token"] {
        assert!(!reason.contains(value));
    }
    assert!(!reason.contains("invalid base domain in the pairing code"));

    let unsupported = uri.replace("v=0", "v=secret-version-private");
    phone.pair(&unsupported).unwrap();
    let view = phone
        .wait_for(|view| matches!(view.pairing, Some(PairingStatus::Failed { .. })))
        .await;
    let Some(PairingStatus::Failed {
        reason,
        message,
        diagnostic,
    }) = view.pairing
    else {
        unreachable!()
    };
    assert_eq!(reason, PairingFailure::Protocol);
    let diagnostic = diagnostic.expect("版本不受支持时仍要保留安全诊断");
    for value in [uri.as_str(), code.secret.expose(), "secret-version-private"] {
        assert!(!message.contains(value));
        assert!(!diagnostic.contains(value));
    }
    assert!(diagnostic.contains("version"));

    phone
        .dispatch(wonton_client::Action::Home {
            action: wonton_protocol::HomeAction::DismissPairingFailure,
        })
        .unwrap();
    assert_server_rejection(&phone, &log_path).await;
}

struct Rejection {
    code: PairingCode,
    query_canary: &'static str,
    query_url: String,
    json_token: &'static str,
    typed_token: &'static str,
}

fn rejection_code(home: &Home) -> Rejection {
    let token = "relay+example";
    let mut code =
        PairingCode::parse(&home.code(Some(wonton_tunnel::pair::RejectReason::Expired))).unwrap();
    code.relays.push(RelayView {
        url: "https://relay.example.com/".parse().unwrap(),
        token: Some(RelayToken::new(token)),
        quic_port: None,
        tier: 0,
    });
    code.relays.push(RelayView {
        url: "https://relay.example.com/".parse().unwrap(),
        token: Some(RelayToken::new("30")),
        quic_port: None,
        tier: 1,
    });
    let query_canary = "query-canary-707";
    let query_url = format!("https://relay.example.com/?access_token={query_canary}");
    code.relays.push(RelayView {
        url: query_url.parse().unwrap(),
        token: None,
        quic_port: None,
        tier: 2,
    });
    // 合法取值各自只出现一种写法：JSON 转义的 `/` 与 `-`、两层百分号后再转义、解码后的 URL 分量。
    let json_token = "json-relay/707";
    let typed_token = "mix/707";
    for (token, tier) in [
        (json_token, 3),
        (typed_token, 4),
        ("\u{1F600}", 7),
        ("pre\u{1F600}post", 8),
    ] {
        code.relays.push(RelayView {
            url: "https://relay.example.com/".parse().unwrap(),
            token: Some(RelayToken::new(token)),
            quic_port: None,
            tier,
        });
    }
    for (url, tier) in [
        ("https://relay.example.com/#frag%2D707", 5),
        ("https://relay.example.com/?access_token=query%252D707", 6),
        ("https://relay.example.com/#frag%20space-preserve707", 9),
    ] {
        code.relays.push(RelayView {
            url: url.parse().unwrap(),
            token: None,
            quic_port: None,
            tier,
        });
    }
    Rejection {
        code,
        query_canary,
        query_url,
        json_token,
        typed_token,
    }
}

async fn assert_server_rejection(phone: &Phone, log_path: &std::path::Path) {
    let home = Home::start().await;
    let Rejection {
        code,
        query_canary,
        query_url,
        json_token,
        typed_token,
    } = rejection_code(&home);
    let token = "relay+example";
    let issued = code.to_uri();
    let secret = PairingCode::parse(&issued)
        .unwrap()
        .secret
        .expose()
        .to_owned();
    let encoded_secret = format!(
        "{}%{:02x}{}",
        &secret[..5],
        secret.as_bytes()[5],
        &secret[6..]
    );
    let colliding_secret = format!("%{:02x}{}", secret.as_bytes()[0], &secret[1..]);
    let encoded_url: String = url::form_urlencoded::byte_serialize(query_url.as_bytes()).collect();
    let variants = [
        token,
        "relay%2Bexample",
        "relay%2bexample",
        "relay%252bexample",
        "%72e%6Cay%2bex%61mple",
        encoded_secret.as_str(),
    ];
    *home.codes.refusal_message.lock().unwrap() = Some(format!(
        "Expired: {issued} {} {colliding_secret} {encoded_url} {}",
        variants.join(" "),
        r#"{"token":"json\u002Drelay\/707"} mix%255Cu002F707 frag-707 query-707 {"token":"relay+example"} ?access_token=query-canary-707"#
    ));
    let mut message = home.codes.refusal_message.lock().unwrap().take().unwrap();
    message.push_str(
        r#" {"e":"\uD83D\uDE00"} {"e":"\uD83d\uDe00"} pre\uD83D\uDE00post {"e":"\uD83D\uDE01"} pre\uD83DXuDE00post frag+space-preserve707 end"#,
    );
    *home.codes.refusal_message.lock().unwrap() = Some(message);
    phone.pair(&issued).unwrap();
    let view = phone
        .wait_for(|view| matches!(view.pairing, Some(PairingStatus::Failed { .. })))
        .await;
    let Some(PairingStatus::Failed {
        reason,
        message,
        diagnostic,
    }) = view.pairing
    else {
        unreachable!()
    };
    assert_eq!(reason, PairingFailure::Expired);
    assert!(!message.contains("Expired"));
    assert!(
        diagnostic
            .as_deref()
            .is_some_and(|raw| raw.contains("Expired"))
    );
    let diagnostic = diagnostic.unwrap();
    let log = std::fs::read_to_string(log_path).unwrap();
    assert_absent(
        &diagnostic,
        &log,
        &[&secret[1..], "relay%252bexample", query_canary],
    );
    // 逃逸与解码后的写法：原始转义、两层百分号叠加转义、fragment/query 解码值，两个 sink 都必须遮住。
    assert_absent(
        &diagnostic,
        &log,
        &[
            json_token,
            typed_token,
            r"json\u002Drelay\/707",
            "mix%255Cu002F707",
            "frag-707",
            "query-707",
            r"\uD83D\uDE00",
            r"\uD83d\uDe00",
        ],
    );
    // 对照：另一个 emoji、非法代理对（缺第二个 `\`）、fragment 里保留的 `+` 不被误遮。
    for kept in [
        "frag+space-preserve707",
        r"pre\uD83DXuDE00post",
        r"\uD83D\uDE01",
    ] {
        assert!(log.contains(kept), "{kept} was wrongly redacted: {log}");
    }
    assert_redacted(&message, &issued, &secret, &variants);
    assert_redacted(&diagnostic, &issued, &secret, &variants);
    assert!(log.contains("the home server rejected pairing"));
    assert_redacted(&log, &issued, &secret, &variants);
    // 对照：规范 JSON、单层 query 的原文也经同一条路径出现，仍须遮住。
    assert!(!log.contains(r#"{"token":"relay+example"}"#));
    assert!(!log.contains("?access_token=query-canary-707"));
}

fn assert_absent(diagnostic: &str, log: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            !diagnostic.contains(needle),
            "{needle} leaked in diagnostics: {diagnostic}"
        );
        assert!(!log.contains(needle), "{needle} leaked in the log");
    }
}

fn assert_redacted(text: &str, issued: &str, secret: &str, variants: &[&str]) {
    assert!(!text.contains(issued), "pairing code leaked");
    assert!(!text.contains(secret), "pairing secret leaked");
    for value in variants {
        assert!(!text.contains(value), "relay token leaked");
    }
}

#[test]
fn decoded_values_beyond_the_layer_budget_redact_the_whole_text() {
    const CHILD: &str = "WONTON_DEEP_DECODE_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "pairing::invalid_details::decoded_values_beyond_the_layer_budget_redact_the_whole_text",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated deep decoding test failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(assert_decoding_over_budget());
}

async fn assert_decoding_over_budget() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("pairing.log");
    wonton_client::init_logging(
        "wonton_client::home::pairing=info".to_owned(),
        Some(log_path.to_string_lossy().into_owned()),
    );
    let phone = Phone::open(dir.path());
    let home = Home::start().await;
    let mut code =
        PairingCode::parse(&home.code(Some(wonton_tunnel::pair::RejectReason::Expired))).unwrap();
    for (url, tier) in [
        ("https://relay.example.com/#frag%2525252Dover707", 0),
        (
            "https://relay.example.com/?access_token=query%252525252Dover707",
            1,
        ),
    ] {
        code.relays.push(RelayView {
            url: url.parse().unwrap(),
            token: None,
            quic_port: None,
            tier,
        });
    }
    let issued = code.to_uri();
    *home.codes.refusal_message.lock().unwrap() =
        Some(format!("Expired: {issued} frag-over707 query-over707"));
    phone.pair(&issued).unwrap();
    let view = phone
        .wait_for(|view| matches!(view.pairing, Some(PairingStatus::Failed { .. })))
        .await;
    let Some(PairingStatus::Failed { diagnostic, .. }) = view.pairing else {
        unreachable!()
    };
    let diagnostic = diagnostic.expect("拒绝诊断仍然保留");
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        diagnostic.contains("[redacted: excessive percent encoding layers]"),
        "{diagnostic}"
    );
    assert_absent(&diagnostic, &log, &["frag-over707", "query-over707"]);
    assert!(log.contains("[redacted: excessive percent encoding layers]"));
}

#[test]
fn duplicate_values_keep_both_matching_forms_regardless_of_relay_order() {
    const CHILD: &str = "WONTON_DUPLICATE_FORM_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "pairing::invalid_details::duplicate_values_keep_both_matching_forms_regardless_of_relay_order",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated duplicate form test failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(assert_duplicate_forms());
}

async fn assert_duplicate_forms() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("pairing.log");
    wonton_client::init_logging(
        "wonton_client::home::pairing=info".to_owned(),
        Some(log_path.to_string_lossy().into_owned()),
    );
    let phone = Phone::open(dir.path());
    let home = Home::start().await;
    let mut code =
        PairingCode::parse(&home.code(Some(wonton_tunnel::pair::RejectReason::Expired))).unwrap();
    // token 在前：query 同值需要留下自己的表单匹配。
    code.relays.push(RelayView {
        url: "https://relay.example.com/".parse().unwrap(),
        token: Some(RelayToken::new("dupe space707")),
        quic_port: None,
        tier: 0,
    });
    code.relays.push(RelayView {
        url: "https://relay.example.com/?access_token=dupe%20space707"
            .parse()
            .unwrap(),
        token: None,
        quic_port: None,
        tier: 1,
    });
    // query 在前：反向顺序同样要命中。
    code.relays.push(RelayView {
        url: "https://relay.example.com/?access_token=dupe2%20space707"
            .parse()
            .unwrap(),
        token: None,
        quic_port: None,
        tier: 2,
    });
    code.relays.push(RelayView {
        url: "https://relay.example.com/".parse().unwrap(),
        token: Some(RelayToken::new("dupe2 space707")),
        quic_port: None,
        tier: 3,
    });
    let issued = code.to_uri();
    *home.codes.refusal_message.lock().unwrap() = Some(format!(
        "Expired: {issued} dupe+space707 and dupe2+space707 end"
    ));
    phone.pair(&issued).unwrap();
    let view = phone
        .wait_for(|view| matches!(view.pairing, Some(PairingStatus::Failed { .. })))
        .await;
    let Some(PairingStatus::Failed { diagnostic, .. }) = view.pairing else {
        unreachable!()
    };
    let diagnostic = diagnostic.expect("拒绝诊断仍然保留");
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert_absent(&diagnostic, &log, &["dupe+space707", "dupe2+space707"]);
    assert!(log.contains(" and "), "{log}");
}
