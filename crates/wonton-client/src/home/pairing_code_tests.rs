use super::*;
use iroh::SecretKey;
use wonton_tunnel::pair::{PairingCode, PairingSecret, SECRET_LEN};

fn pairing_code() -> PairingCode {
    PairingCode {
        server_id: SecretKey::from_bytes(&[7; 32]).public(),
        relays: Vec::new(),
        direct_addrs: vec!["192.0.2.10:18482".parse().unwrap()],
        base_domain: "wonton.localhost".into(),
        core_url: "http://wonton.localhost:18480/".parse().unwrap(),
        secret: PairingSecret::from_bytes([42; SECRET_LEN]),
    }
}

#[test]
fn invalid_pairing_codes_are_localized_without_exposing_input() {
    let code = pairing_code();
    let uri = code.to_uri();
    let bad_domain = uri.replace("domain=wonton.localhost", "domain=bad%2Ftoken-value");
    let invalid = PairingCode::parse(&bad_domain).unwrap_err();
    let version = PairingCode::parse(&uri.replace("v=0", "v=secret-version-value")).unwrap_err();
    let mut summaries = Vec::new();
    for tag in ["en", "zh-Hans", "zh-Hant", "ja"] {
        let lang = CATALOG.negotiate(&[tag]);
        let text = summary(&invalid, lang);
        assert!(!text.contains("bad/token-value"), "{tag}: {text}");
        assert!(
            !text.contains("invalid base domain in the pairing code"),
            "{tag}: {text}"
        );
        assert!(
            summaries.iter().all(|other| other != &text),
            "{tag}: {text}"
        );
        summaries.push(text);
        let rejected = rejection(RejectReason::Expired, lang);
        assert!(!rejected.contains("Expired"), "{tag}: {rejected}");
        assert!(!rejected.is_empty());
    }
    for diagnostic in [diagnostic(&invalid), diagnostic(&version)] {
        assert!(!diagnostic.contains("token-value"), "{diagnostic}");
        assert!(!diagnostic.contains("secret-version-value"), "{diagnostic}");
        assert!(!diagnostic.contains(code.secret.expose()), "{diagnostic}");
        assert!(!diagnostic.contains(&uri), "{diagnostic}");
    }
    assert!(diagnostic(&invalid).contains("base domain"));
    assert!(
        diagnostic(&InvalidCode::Malformed {
            field: "server ID",
            reason: "missing id".into(),
        })
        .contains("missing id")
    );
}
