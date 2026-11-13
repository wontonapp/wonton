//! 管理接口的文字按请求的 `Accept-Language` 写（ADR-0043）。

use hyper::StatusCode;

use super::tests::{CORE, Harness};
use crate::l10n::CATALOG;

/// 401 的文字按请求的 Accept-Language 写（ADR-0043）；期望值由目录格式化出来比。
#[tokio::test]
async fn the_denied_message_follows_the_accept_language_header() {
    let h = Harness::start(None).await;
    let ja = h
        .get(CORE, "/_wonton/api/plugins", &[("accept-language", "ja")])
        .await;
    assert_eq!(ja.status, StatusCode::UNAUTHORIZED);
    let fr = h
        .get(
            CORE,
            "/_wonton/api/plugins",
            &[("accept-language", "fr-FR")],
        )
        .await;
    assert_eq!(fr.status, StatusCode::UNAUTHORIZED);
    // fr-FR 目录里没有，回退英文；两种头要真的给出两种话。
    let expected = |tag: &str| CATALOG.format(CATALOG.negotiate(&[tag]), "gateway-denied", &[]);
    assert_eq!(ja.json()["error"]["message"], expected("ja"));
    assert_eq!(fr.json()["error"]["message"], expected("en"));
    assert_ne!(expected("ja"), expected("en"));
}

/// 设备的语言记在设备归属的人名下（ADR-0043）：主人的和家人的各记各的，互不覆盖。
#[tokio::test]
async fn device_languages_are_recorded_under_the_person() {
    use super::family_tests::send_as;
    use crate::members::MemberRecord;
    use wonton_protocol::MemberStatus;

    let h = Harness::start(None).await;
    h.members
        .invite(MemberRecord {
            username: "alice".into(),
            user_id: "@alice:wonton.localhost".into(),
            display_name: "爱丽丝".into(),
            email: None,
            status: MemberStatus::Joined,
            role_id: "friend".to_owned(),
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
    let owner_device = iroh::SecretKey::generate().public();
    h.tunnel.devices.add(owner_device, None).unwrap();
    let alice_device = iroh::SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(alice_device, None, Some("alice".into()), |_| true)
        .unwrap();

    let path = "/_wonton/api/devices";
    for (device, language) in [(owner_device, "zh-Hans-CN, en;q=0.8"), (alice_device, "ja")] {
        let reply = send_as(
            &h.app,
            Some(device),
            hyper::Method::GET,
            CORE,
            path,
            &[("accept-language", language)],
            "",
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
    }

    let stored: std::collections::BTreeMap<String, String> =
        crate::plugin::store::read_people_languages(
            &h.data_dir.join("plugins/store/people-languages.json"),
        );
    assert_eq!(
        stored.get("owner").map(String::as_str),
        Some("zh-Hans-CN, en;q=0.8")
    );
    assert_eq!(stored.get("alice").map(String::as_str), Some("ja"));
}
