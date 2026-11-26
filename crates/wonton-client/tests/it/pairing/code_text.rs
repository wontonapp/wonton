use iroh::SecretKey;
use serde_json::json;
use wonton_client::{CoreError, PairingFailure};
use wonton_tunnel::pair::SECRET_LEN;

use super::support::{Home, Phone};

/// 中继地址写成 http：码在本机就解析不过，当场报“要用 https”，不停在配对页等满时限。
#[tokio::test]
async fn a_code_with_an_http_relay_is_rejected_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let id = SecretKey::generate().public();
    let relays =
        serde_json::to_string(&json!([{ "url": "http://127.0.0.1:19194/", "tier": 0 }])).unwrap();
    let code = format!(
        "wonton://pair?v=0&id={id}&relays={}&addr=127.0.0.1:1&domain=wonton.localhost&core=http%3A%2F%2Fwonton.localhost%3A1%2F&secret={}",
        url::form_urlencoded::byte_serialize(relays.as_bytes()).collect::<String>(),
        "42".repeat(SECRET_LEN),
    );
    let error = phone.pair(&code).unwrap_err();
    let text = format!("{error:?}");
    assert!(text.to_ascii_lowercase().contains("https"), "{text}");
}

#[tokio::test]
async fn other_qr_codes_are_rejected_and_newer_codes_ask_for_an_update() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let before = phone.home();
    for text in ["", "https://example.com/", "wonton://pair?v=0"] {
        assert!(
            matches!(phone.pair(text), Err(CoreError::InvalidInput { .. })),
            "{text}"
        );
    }
    assert_eq!(phone.home(), before);

    let home = Home::start().await;
    let newer = home.code(None).replace("v=0", "v=9");
    phone.pair(&newer).unwrap();
    let (failure, message) = phone.pairing_failure().await;
    assert_eq!(failure, PairingFailure::Protocol, "{message}");
}
