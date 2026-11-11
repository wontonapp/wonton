use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

use super::*;
use crate::{ShareFailure, ShareProblem};

fn round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    assert_eq!(
        serde_json::from_value::<T>(serde_json::to_value(value).unwrap()).unwrap(),
        *value
    );
}

#[test]
fn discovery_and_authentication_preserve_wire_fields() {
    let discovery = WontonDiscovery {
        endpoint_id: "aa".repeat(32),
        relay_urls: vec!["https://relay.example.org/".into()],
    };
    round_trip(&discovery);
    let request = ShareRequest::Authenticate {
        openid: MatrixOpenIdToken {
            access_token: SecretString::new("openid-secret"),
            matrix_server_name: "matrix.org".into(),
            expires_in: 3600,
        },
    };
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(
        wire,
        json!({
            "type": "authenticate",
            "openid": {"access_token":"openid-secret", "matrix_server_name":"matrix.org", "expires_in":3600},
        })
    );
    round_trip(&request);
    assert!(!format!("{request:?}").contains("openid-secret"));
    round_trip(&ShareRequest::List);
    round_trip(&ShareReply::Shares {
        shares: SharedPlugins {
            user_id: "@bob:matrix.org".into(),
            plugins: vec![],
        },
    });
}

#[test]
fn incomplete_or_wrongly_typed_tokens_are_not_authentication() {
    for token in [
        json!({"matrix_server_name":"matrix.org", "expires_in":3600}),
        json!({"access_token":"secret", "expires_in":3600}),
        json!({"access_token":"secret", "matrix_server_name":"matrix.org"}),
        json!({"access_token":"secret", "matrix_server_name":"matrix.org", "expires_in":-1}),
        json!({"access_token":"secret", "matrix_server_name":"matrix.org", "expires_in":"3600"}),
    ] {
        assert!(
            serde_json::from_value::<ShareRequest>(json!({"type":"authenticate", "openid":token}))
                .is_err()
        );
    }
}

#[test]
fn share_refusals_keep_header_names_and_identity_causes() {
    for refusal in [
        GatewayRefusal::ShareIdentity,
        GatewayRefusal::ShareRequired,
        GatewayRefusal::ShareRevoked,
        GatewayRefusal::ShareExpired,
    ] {
        let identity = (refusal == GatewayRefusal::ShareIdentity)
            .then_some(ShareIdentityFailure::ServerNameMismatch);
        round_trip(&ShareReply::Refused { refusal, identity });
        let wire = serde_json::to_value(refusal).unwrap();
        assert_eq!(wire, json!(refusal.as_str()));
        assert_eq!(
            serde_json::from_value::<GatewayRefusal>(wire).unwrap(),
            refusal
        );
    }
    assert_eq!(
        serde_json::from_value::<GatewayRefusal>(json!("new_share_refusal")).unwrap(),
        GatewayRefusal::Other
    );
    assert_eq!(
        serde_json::from_value::<ShareRequest>(json!({"type":"future_request", "payload":{}}))
            .unwrap(),
        ShareRequest::Other
    );
    assert_eq!(
        serde_json::from_value::<ShareReply>(json!({"type":"future_reply"})).unwrap(),
        ShareReply::Other
    );
    let reply: ShareReply =
        serde_json::from_value(json!({"type":"refused", "refusal":"future", "identity":null}))
            .unwrap();
    assert_eq!(
        reply,
        ShareReply::Refused {
            refusal: GatewayRefusal::Other,
            identity: None
        }
    );
    round_trip(&ShareProblem {
        reason: ShareFailure::Identity,
        http_status: Some(401),
        identity: Some(ShareIdentityFailure::Other),
    });
}
