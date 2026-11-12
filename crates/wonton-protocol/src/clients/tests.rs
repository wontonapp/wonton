//! 客户端接口类型的往返和向后兼容：核心比 App 新时，枚举落到 `Other`，旧的请求体照旧解析。

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use super::*;
use crate::SecretString;

/// 双向：JSON 解析成类型，类型再写回同一份 JSON。
fn round_trip<T>(value: serde_json::Value, expected: T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let decoded: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(decoded, expected);
    assert_eq!(serde_json::to_value(&decoded).unwrap(), value);
}

/// 只解析：未知取值落到 `Other`。
fn parses_as_other<T>(value: serde_json::Value)
where
    T: DeserializeOwned + PartialEq + std::fmt::Debug,
    T: FromOther,
{
    let decoded: T = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, T::other());
}

/// 给每个带 `Other` 的枚举取 `other` 那一个，免得测试里逐个写路径。
trait FromOther {
    fn other() -> Self;
}

fn password_summary() -> PasswordSummary {
    PasswordSummary {
        id: "pw1".into(),
        label: "家里的 iPad".into(),
        created_by_device: Some("iPad".into()),
        last_viewed_by: vec!["Mac".into()],
        pending: Some(PendingOp::Revoke),
    }
}

fn plugin() -> ClientsPlugin {
    ClientsPlugin {
        id: "app.immich.photos".into(),
        label: "Immich".into(),
        lan_url: Some("https://immich.lan.example.test:8483".into()),
        public_url: None,
        entries: vec![
            EntrySwitch {
                kind: EntryKind::Lan,
                enabled: true,
            },
            EntrySwitch {
                kind: EntryKind::Tcp { name: "ssh".into() },
                enabled: false,
            },
        ],
        auth: ClientAuthKind::Credentials,
        username: Some("owner".into()),
        passwords: vec![password_summary()],
        public_rules: Some(PublicRules {
            mode: PublicRuleMode::Allow,
            paths: vec!["/webhook".into()],
        }),
        blocked: Some(EntryBlock::PublicAccessOff),
    }
}

fn domain() -> DomainSettings {
    DomainSettings {
        suffix: Some("example.test".into()),
        suffix_editable: true,
        provider: Some("desec".into()),
        credentials_set: true,
        terms_accepted: true,
        certificate: CertificateState::Renewing {
            not_after_ms: 1_800_000_000_000,
            error: "propagation timed out".into(),
        },
        caa: CaaState::Missing,
        lan_record: LanRecordState::Wrong {
            found: "192.0.2.1".into(),
        },
    }
}

fn ingress() -> IngressSettings {
    IngressSettings {
        kind: Some(IngressKind::OwnFrps),
        port: Some(8443),
        server_addr: Some("vps.example.test".into()),
        server_port: Some(7000),
        server_name: Some("frps.example.test".into()),
        ca_pem: Some("PEM".into()),
        token_set: true,
    }
}

fn gates() -> PublicSwitchState {
    PublicSwitchState {
        connector: GateState::Done,
        crt_list: GateState::Failed {
            detail: "entry 没在运行".into(),
        },
        core: GateState::Unknown,
    }
}

fn status() -> ClientsStatus {
    ClientsStatus {
        core_in_container: true,
        domain: domain(),
        ingress: ingress(),
        public_access: PublicAccess { enabled: false },
        gates: gates(),
        self_probe: ProbeResult::ConnectorDown,
        public_reach: PublicReach::PublicAccessOff,
        public_now: vec![PublicItem {
            plugin_id: "app.n8n".into(),
            label: "n8n".into(),
            rules: Some(PublicRules {
                mode: PublicRuleMode::Deny,
                paths: vec!["/rest/login".into()],
            }),
            guest_links: vec!["聚会".into()],
        }],
        bans: vec![Ban {
            source: "2001:db8::/64".into(),
            since_ms: 1,
            until_ms: 2,
            failures: 10,
        }],
        pending: vec![PendingChange {
            kind: PendingChangeKind::CredentialRotate,
            plugin_id: Some("app.immich.photos".into()),
            detail: None,
        }],
    }
}

#[test]
fn every_response_round_trips() {
    round_trip(json!([plugin_serde()]), vec![plugin()]);
    round_trip(
        serde_json::to_value(PasswordValue {
            username: "owner".into(),
            password: SecretString::new("SECRET"),
        })
        .unwrap(),
        PasswordValue {
            username: "owner".into(),
            password: SecretString::new("SECRET"),
        },
    );
    round_trip(serde_json::to_value(domain()).unwrap(), domain());
    round_trip(serde_json::to_value(ingress()).unwrap(), ingress());
    round_trip(
        serde_json::to_value(GuestLink {
            id: "g1".into(),
            label: "聚会".into(),
            expires_at_ms: 3,
            path: Some("/room".into()),
            last_used_at_ms: Some(2),
            url: Some("https://scribble.example.test/_wonton/guest/T".into()),
        })
        .unwrap(),
        GuestLink {
            id: "g1".into(),
            label: "聚会".into(),
            expires_at_ms: 3,
            path: Some("/room".into()),
            last_used_at_ms: Some(2),
            url: Some("https://scribble.example.test/_wonton/guest/T".into()),
        },
    );
    round_trip(serde_json::to_value(status()).unwrap(), status());
}

/// `plugin()` 的 JSON，用记录的写法手写一遍，防止字段名悄悄改掉。
fn plugin_serde() -> serde_json::Value {
    json!({
        "id": "app.immich.photos", "label": "Immich",
        "lan_url": "https://immich.lan.example.test:8483", "public_url": null,
        "entries": [
            {"kind": {"kind": "lan"}, "enabled": true},
            {"kind": {"kind": "tcp", "name": "ssh"}, "enabled": false},
        ],
        "auth": "credentials", "username": "owner",
        "passwords": [{
            "id": "pw1", "label": "家里的 iPad", "created_by_device": "iPad",
            "last_viewed_by": ["Mac"], "pending": "revoke",
        }],
        "public_rules": {"mode": "allow", "paths": ["/webhook"]},
        "blocked": "public_access_off",
    })
}

#[test]
fn every_request_round_trips() {
    round_trip(json!({"enabled": true}), SetEntryRequest { enabled: true });
    round_trip(
        json!({"label": "家里的 iPad"}),
        NewPasswordRequest {
            label: "家里的 iPad".into(),
        },
    );
    round_trip(
        json!({
            "suffix": "example.test", "provider": "desec",
            "credentials": [{"name": "DESEC_TOKEN", "value": "SECRET"}],
            "accept_terms": true,
        }),
        PutDomainRequest {
            suffix: Some("example.test".into()),
            provider: "desec".into(),
            credentials: vec![CredentialEntry {
                name: "DESEC_TOKEN".into(),
                value: SecretString::new("SECRET"),
            }],
            accept_terms: true,
        },
    );
    round_trip(
        json!({"kind": "router_forward", "port": 8443}),
        PutIngressRequest::RouterForward { port: 8443 },
    );
    round_trip(
        json!({
            "kind": "own_frps", "server_addr": "vps.example.test", "server_port": 7000,
            "server_name": "frps.example.test", "token": "SECRET", "ca_pem": "PEM",
        }),
        PutIngressRequest::OwnFrps {
            server_addr: "vps.example.test".into(),
            server_port: 7000,
            server_name: "frps.example.test".into(),
            token: SecretString::new("SECRET"),
            ca_pem: "PEM".into(),
        },
    );
    round_trip(
        json!({"kind": "cloudflare_tunnel", "token": "SECRET"}),
        PutIngressRequest::CloudflareTunnel {
            token: SecretString::new("SECRET"),
        },
    );
    round_trip(json!({"enabled": false}), PublicAccess { enabled: false });
    round_trip(
        json!({"label": "聚会", "ttl_seconds": 3600, "path": "/room"}),
        NewGuestLinkRequest {
            label: "聚会".into(),
            ttl_seconds: 3600,
            path: Some("/room".into()),
        },
    );
    round_trip(
        json!({"keep_shared_passwords": true}),
        RevokeDeviceRequest {
            keep_shared_passwords: true,
        },
    );
}

#[test]
fn old_device_revoke_request_still_parses() {
    let request: RevokeDeviceRequest = serde_json::from_value(json!({})).unwrap();
    assert!(!request.keep_shared_passwords);
}

#[test]
fn unknown_kinds_are_other() {
    parses_as_other::<EntryKind>(json!({"kind": "future"}));
    parses_as_other::<ClientAuthKind>(json!("future"));
    parses_as_other::<EntryBlock>(json!("future"));
    parses_as_other::<PublicRuleMode>(json!("future"));
    parses_as_other::<PendingOp>(json!("future"));
    parses_as_other::<CertificateState>(json!({"state": "future"}));
    parses_as_other::<CaaState>(json!("future"));
    parses_as_other::<LanRecordState>(json!({"state": "future"}));
    parses_as_other::<IngressKind>(json!("future"));
    parses_as_other::<GateState>(json!({"state": "future"}));
    parses_as_other::<PublicReach>(json!("future"));
    parses_as_other::<ProbeResult>(json!("future"));
    parses_as_other::<PendingChangeKind>(json!("future"));
    parses_as_other::<ClientsFailure>(json!("future"));
}

#[test]
fn entry_kinds_map_to_their_path_segment() {
    assert_eq!(EntryKind::Lan.as_path_segment(), Some("lan"));
    assert_eq!(EntryKind::Public.as_path_segment(), Some("public"));
    assert_eq!(
        EntryKind::Tcp { name: "ssh".into() }.as_path_segment(),
        Some("ssh")
    );
    assert_eq!(EntryKind::Other.as_path_segment(), None);
}

#[test]
fn paths_are_stable() {
    assert_eq!(CLIENTS_HEADER, "X-Wonton-Clients");
    assert_eq!(CLIENTS_ERROR_HEADER, "X-Wonton-Error");
    assert_eq!(clients_path(), "/_wonton/clients");
    assert_eq!(
        clients_entry_path("app.immich.photos", "lan"),
        "/_wonton/clients/app.immich.photos/entries/lan"
    );
    assert_eq!(
        clients_passwords_path("app.immich.photos"),
        "/_wonton/clients/app.immich.photos/passwords"
    );
    assert_eq!(
        clients_password_path("app.immich.photos", "pw1"),
        "/_wonton/clients/app.immich.photos/passwords/pw1"
    );
    assert_eq!(clients_domain_path(), "/_wonton/clients/domain");
    assert_eq!(dns_providers_path(), "/_wonton/clients/domain/providers");
    assert_eq!(clients_ingress_path(), "/_wonton/clients/ingress");
    assert_eq!(
        clients_public_access_path(),
        "/_wonton/clients/public-access"
    );
    assert_eq!(
        clients_guests_path("app.scribble"),
        "/_wonton/clients/app.scribble/guests"
    );
    assert_eq!(
        clients_guest_path("app.scribble", "g1"),
        "/_wonton/clients/app.scribble/guests/g1"
    );
    assert_eq!(clients_status_path(), "/_wonton/clients/status");
    assert_eq!(
        clients_ban_path("2001:db8::/64"),
        "/_wonton/clients/bans/2001:db8::/64"
    );
}

#[test]
fn failures_keep_their_status_and_wire_name() {
    for (failure, status) in [
        (ClientsFailure::Unauthenticated, Some(401)),
        (ClientsFailure::PluginUnavailable, Some(503)),
        (ClientsFailure::CredentialsFailed, Some(502)),
        (ClientsFailure::InvalidInput, Some(400)),
        (ClientsFailure::Conflict, Some(409)),
        (ClientsFailure::LanUnavailable, Some(422)),
        (ClientsFailure::PortQuotaExceeded, Some(409)),
        (ClientsFailure::NotRegistered, Some(409)),
        (ClientsFailure::SetupPending, Some(409)),
        (ClientsFailure::CoreNotInContainer, Some(422)),
        (ClientsFailure::EntryClosed, Some(409)),
        (ClientsFailure::IngressUnavailable, Some(503)),
        (ClientsFailure::PublicAccessOff, Some(409)),
        (ClientsFailure::Other, None),
    ] {
        assert_eq!(failure.http_status(), status, "{failure:?}");
        assert_eq!(
            serde_json::to_value(failure).unwrap(),
            json!(failure.as_str()),
            "{failure:?}"
        );
    }
    round_trip(
        json!({"error": {"kind": "conflict", "message": "先发一次密码"}}),
        ClientsErrorBody {
            error: ClientsProblem {
                kind: ClientsFailure::Conflict,
                message: "先发一次密码".into(),
            },
        },
    );
}

#[test]
fn guest_ttl_constants() {
    assert_eq!(GUEST_TTL_DEFAULT, 4 * 60 * 60);
    assert_eq!(GUEST_TTL_MAX, 7 * 24 * 60 * 60);
    assert_eq!(GUEST_LINKS_MAX, 20);
}

macro_rules! from_other {
    ($($type:ty => $variant:expr),* $(,)?) => {$(
        impl FromOther for $type {
            fn other() -> Self {
                $variant
            }
        }
    )*};
}

from_other! {
    EntryKind => EntryKind::Other,
    ClientAuthKind => ClientAuthKind::Other,
    EntryBlock => EntryBlock::Other,
    PublicRuleMode => PublicRuleMode::Other,
    PendingOp => PendingOp::Other,
    CertificateState => CertificateState::Other,
    CaaState => CaaState::Other,
    LanRecordState => LanRecordState::Other,
    IngressKind => IngressKind::Other,
    GateState => GateState::Other,
    PublicReach => PublicReach::Other,
    ProbeResult => ProbeResult::Other,
    PendingChangeKind => PendingChangeKind::Other,
    ClientsFailure => ClientsFailure::Other,
}
