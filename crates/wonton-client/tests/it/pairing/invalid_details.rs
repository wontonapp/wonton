use std::process::Command;

use iroh::SecretKey;
use url::Url;
use wonton_client::{CoreError, PairingFailure, PairingStatus};
use wonton_tunnel::pair::{PairingCode, PairingSecret, SECRET_LEN};
use wonton_tunnel::relays::{Relay, RelayToken};

use super::{Home, Phone};

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
    let home = Home::start().await;
    let token = "relay+example";
    let mut code =
        PairingCode::parse(&home.code(Some(wonton_tunnel::pair::RejectReason::Expired))).unwrap();
    code.relays.push(Relay {
        url: "https://relay.example.com/".parse().unwrap(),
        token: Some(RelayToken::new(token)),
        tier: 0,
    });
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
    let variants = [
        token,
        "relay%2Bexample",
        "relay%2bexample",
        "%72e%6Cay%2bex%61mple",
        encoded_secret.as_str(),
    ];
    *home.codes.refusal_message.lock().unwrap() =
        Some(format!("Expired: {issued} {}", variants.join(" ")));
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
    assert_redacted(&message, &issued, &secret, &variants);
    assert_redacted(&diagnostic, &issued, &secret, &variants);
    let log = std::fs::read_to_string(log_path).unwrap();
    assert!(log.contains("the home server rejected pairing"));
    assert_redacted(&log, &issued, &secret, &variants);
}

fn assert_redacted(text: &str, issued: &str, secret: &str, variants: &[&str]) {
    assert!(!text.contains(issued), "pairing code leaked");
    assert!(!text.contains(secret), "pairing secret leaked");
    for value in variants {
        assert!(!text.contains(value), "relay token leaked");
    }
}
