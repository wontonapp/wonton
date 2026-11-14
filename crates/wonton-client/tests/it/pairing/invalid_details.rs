use iroh::SecretKey;
use url::Url;
use wonton_client::{CoreError, PairingFailure, PairingStatus};
use wonton_tunnel::pair::{PairingCode, PairingSecret, SECRET_LEN};

use super::{Home, Phone};

#[tokio::test]
async fn invalid_pairing_input_and_server_rejection_keep_diagnostics_secret_safe() {
    let dir = tempfile::tempdir().unwrap();
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
    let issued = home.code(Some(wonton_tunnel::pair::RejectReason::Expired));
    let secret = PairingCode::parse(&issued)
        .unwrap()
        .secret
        .expose()
        .to_owned();
    *home.codes.refusal_message.lock().unwrap() = Some(format!("Expired: {issued}"));
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
    assert!(!diagnostic.contains(&issued));
    assert!(!diagnostic.contains(&secret));
}
