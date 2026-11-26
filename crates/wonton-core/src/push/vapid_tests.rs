use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hyper::Uri;
use web_push_native::p256::SecretKey;
use web_push_native::p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};

use super::Vapid;

fn key_text() -> String {
    let secret = SecretKey::random(&mut web_push_native::p256::elliptic_curve::rand_core::OsRng);
    URL_SAFE_NO_PAD.encode(secret.to_bytes())
}

fn claims(jwt: &str) -> serde_json::Value {
    let payload = jwt.split('.').nth(1).unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
}

#[test]
fn signs_an_es256_token_for_the_endpoint_origin() {
    let vapid = Vapid::from_key(&key_text(), "mailto:ops@example.com".to_owned()).unwrap();
    let header = vapid
        .header(
            &Uri::from_static("https://push.example/push/abc"),
            Duration::from_secs(3600),
        )
        .unwrap();
    let text = header.to_str().unwrap();
    let (token, public) = text
        .strip_prefix("vapid t=")
        .unwrap()
        .split_once(", k=")
        .unwrap();
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT 有三段");
    let head: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    assert_eq!(head["alg"], "ES256");

    let k = URL_SAFE_NO_PAD.decode(public).unwrap();
    assert_eq!(k.len(), 65);
    assert_eq!(k[0], 0x04, "公钥是未压缩点");
    let verifier = VerifyingKey::from_sec1_bytes(&k).unwrap();
    let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
    verifier
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("签名能被自己的公钥验过");

    let claims = claims(token);
    assert_eq!(
        claims["aud"], "https://push.example",
        "aud 只到源，不带路径和端口"
    );
    assert_eq!(claims["sub"], "mailto:ops@example.com");
    assert!(claims["exp"].as_i64().unwrap() > 3000);
}

#[test]
fn keeps_port_out_of_the_audience() {
    let vapid = Vapid::from_key(&key_text(), "mailto:ops@example.com".to_owned()).unwrap();
    let header = vapid
        .header(
            &Uri::from_static("http://127.0.0.1:8790/push/abc"),
            Duration::from_secs(60),
        )
        .unwrap();
    let token = header.to_str().unwrap().split(' ').nth(1).unwrap();
    let token = token.strip_prefix("t=").unwrap().split(',').next().unwrap();
    assert_eq!(claims(token)["aud"], "http://127.0.0.1");
}

#[test]
fn rejects_text_that_is_not_a_key() {
    let error =
        Vapid::from_key(&URL_SAFE_NO_PAD.encode(b"hello"), "mailto:a@b".to_owned()).unwrap_err();
    assert!(error.contains("要 32 字节"), "{error}");
}

#[test]
fn rejects_text_that_is_not_base64() {
    let error = Vapid::from_key("!!!", "mailto:a@b".to_owned()).unwrap_err();
    assert!(error.contains("不是 base64url"), "{error}");
}
