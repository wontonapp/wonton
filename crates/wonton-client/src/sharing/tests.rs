use super::*;
use wonton_protocol::{ProxySettings, SharedAppRow, SharedPlugin, TabDisplay, TabState};
use wonton_tunnel::sharing::{ShareError, ShareRefusal};

fn friend(id: &str) -> wonton_matrix::FriendSource {
    wonton_matrix::FriendSource {
        user_id: id.try_into().unwrap(),
        display_name: None,
    }
}

fn ready_source(id: &str) -> Source {
    let mut source = Source::new(&friend(id));
    source.view.apps.push(SharedAppRow {
        plugin: SharedPlugin {
            plugin_id: "app.same".into(),
            title: id.into(),
            web_url: "http://app-same.wonton.localhost/".into(),
            display: TabDisplay::Standard,
            state: TabState::Ready,
        },
        problem: None,
    });
    source.view.state = SharedAppsState::Ready {
        proxy: ProxySettings {
            host: "127.0.0.1".into(),
            port: 1,
            username: id.into(),
            password: "test".into(),
            domain: "wonton.localhost".into(),
        },
    };
    source
}

#[test]
fn losing_one_source_preserves_its_apps_without_borrowing_another_proxy() {
    let view = watch::Sender::new(PluginsViewModel::default());
    let mut sources = HashMap::from([
        ("@alice:a.test".into(), ready_source("@alice:a.test")),
        ("@bob:b.test".into(), ready_source("@bob:b.test")),
    ]);
    sources
        .get_mut("@alice:a.test")
        .unwrap()
        .fail(problem(ShareFailure::Revoked, Some(403)));
    publish(&view, &sources);
    let shared = view.borrow().shared_apps.clone().unwrap();
    assert_eq!(shared.sources.len(), 2);
    assert_eq!(shared.sources[0].apps.len(), 1);
    assert_eq!(shared.sources[1].apps.len(), 1);
    assert!(
        matches!(&shared.sources[0].state, SharedAppsState::Failed { problem } if problem.reason == ShareFailure::Revoked)
    );
    assert!(!matches!(
        shared.sources[1].state,
        SharedAppsState::Ready { .. }
    ));
    sources.clear();
    publish(&view, &sources);
    assert!(
        view.borrow()
            .shared_apps
            .as_ref()
            .unwrap()
            .sources
            .is_empty()
    );
}

#[test]
fn stale_connection_snapshot_never_republishes_ready() {
    let source = ready_source("@alice:a.test");
    assert!(matches!(
        source.snapshot().state,
        SharedAppsState::Failed { .. }
    ));
    assert_eq!(source.snapshot().apps.len(), 1);
}

#[test]
fn list_refusals_preserve_identity_and_authorization_categories() {
    use wonton_protocol::{GatewayRefusal, ShareIdentityFailure};
    for (refusal, reason, status) in [
        (GatewayRefusal::ShareIdentity, ShareFailure::Identity, 401),
        (GatewayRefusal::ShareRequired, ShareFailure::Required, 403),
        (GatewayRefusal::ShareRevoked, ShareFailure::Revoked, 403),
        (GatewayRefusal::ShareExpired, ShareFailure::Expired, 403),
    ] {
        let identity =
            (reason == ShareFailure::Identity).then_some(ShareIdentityFailure::ServerNameMismatch);
        let result =
            source::transport_problem(ShareError::Refused(ShareRefusal { refusal, identity }));
        assert_eq!(result.reason, reason);
        assert_eq!(result.http_status, Some(status));
        assert_eq!(result.identity, identity);
    }
    assert_eq!(
        source::transport_problem(ShareError::Timeout).reason,
        ShareFailure::Unreachable
    );
}

#[test]
fn invalid_endpoint_or_credentialed_relay_cannot_become_a_connection_target() {
    let id = iroh::SecretKey::generate().public().to_string();
    for (endpoint_id, relay_urls) in [
        ("not-an-endpoint".into(), vec![]),
        (id.clone(), vec!["http://relay.test".into()]),
        (id.clone(), vec!["https://name:secret@relay.test".into()]),
    ] {
        assert!(
            discovery::validate(wonton_protocol::WontonDiscovery {
                endpoint_id,
                relay_urls
            })
            .is_err()
        );
    }
    assert!(
        discovery::validate(wonton_protocol::WontonDiscovery {
            endpoint_id: id,
            relay_urls: vec![]
        })
        .is_ok()
    );
}

#[tokio::test]
async fn gateway_refusal_probe_is_not_plugin_content_success() {
    use axum::{
        Router,
        http::{HeaderMap, StatusCode},
        routing::head,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/",
                head(|| async {
                    let mut headers = HeaderMap::new();
                    headers.insert("x-wonton-error", "share_expired".parse().unwrap());
                    (StatusCode::FORBIDDEN, headers)
                }),
            ),
        )
        .await
        .unwrap();
    });
    let mut source = ready_source("@alice:a.test");
    source.view.state = SharedAppsState::Ready {
        proxy: ProxySettings {
            host: address.ip().to_string(),
            port: address.port(),
            username: "test".into(),
            password: "test".into(),
            domain: "wonton.localhost".into(),
        },
    };
    source.probe("app.same").await;
    assert_eq!(
        source.view.apps[0].problem.as_ref().unwrap().reason,
        ShareFailure::Expired
    );
    assert_eq!(
        source.view.apps[0].problem.as_ref().unwrap().http_status,
        Some(403)
    );
    server.abort();
}
