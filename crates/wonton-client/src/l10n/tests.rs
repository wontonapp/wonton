use std::net::Ipv4Addr;
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, SecretKey};
use wonton_protocol::{
    Action, CoreError, DeviceInvite, HomeAction, HomeFailure, HomeStatus, SettingsAction,
};
use wonton_tunnel::{Allowlist, TunnelServer};

use super::{CATALOG, RESOURCES};
use crate::{ClientCore, HomeSubscription, HomeViewModel, Network};

#[test]
fn client_catalogs_are_consistent() {
    wonton_l10n::check_catalog(RESOURCES).unwrap();
}

#[test]
fn duplicate_key_across_group_files_fails_the_catalog_check() {
    // 各组文件拼成一份语言资源：同一个键出现在两组里，要在检查时报错，不能靠后一个悄悄盖掉。
    let error = wonton_l10n::check_catalog(&[("en", "files-save = Save\nfiles-save = Save it\n")])
        .expect_err("同一个键写了两遍");
    assert!(
        error.iter().any(|line| line.contains("files-save")),
        "{error:?}"
    );
}

#[tokio::test]
async fn set_languages_localizes_invite_details_and_core_error_prefixes() {
    let dir = tempfile::tempdir().unwrap();
    let client =
        ClientCore::with_network(dir.path().to_str().unwrap(), Network::DirectOnly).unwrap();
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/_wonton/api/pairing-codes",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({
                        "error": { "kind": "pairing_closed_42", "message": "upstream 原文 / 503" }
                    })),
                )
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let device = client.home().device_id.parse().unwrap();
    let addr = endpoint.bound_sockets()[0];
    let id = endpoint.id();
    let server = TunnelServer::spawn(endpoint, Allowlist::new([device]), address);
    client
        .dispatch(Action::Home {
            action: HomeAction::ConnectHome {
                ticket: serde_json::json!({
                    "server": { "endpoint_id": id, "direct_addrs": [addr] },
                    "base_domain": "wonton.localhost",
                    "core_url": "http://wonton.localhost:18480/"
                })
                .to_string(),
            },
        })
        .unwrap();
    let subscription = client.subscribe_home();
    wait_home(&subscription, |view| {
        view.status == HomeStatus::Connected && view.proxy.is_some()
    })
    .await;
    assert_language_invites(&client, &subscription).await;
    server.shutdown().await;
    upstream.abort();
    drop(client);
}

async fn assert_language_invites(client: &ClientCore, subscription: &HomeSubscription) {
    for languages in [
        vec!["ja"],
        vec!["fr-FR"],
        vec!["en", "ja"],
        vec!["zh-TW"],
        vec!["zh-SG"],
        vec![],
    ] {
        let lang = CATALOG.negotiate(&languages[..languages.len().min(1)]);
        client
            .dispatch(Action::Settings {
                action: SettingsAction::SetLanguages {
                    languages: languages.iter().map(|tag| (*tag).to_owned()).collect(),
                },
            })
            .unwrap();
        client
            .dispatch(Action::Home {
                action: HomeAction::InviteDevice,
            })
            .unwrap();
        let view = wait_home(subscription, |view| {
            matches!(view.invite, Some(DeviceInvite::Failed { .. }))
        })
        .await;
        let Some(DeviceInvite::Failed {
            reason,
            message: detail,
        }) = view.invite
        else {
            unreachable!()
        };
        let upstream_detail = wonton_l10n::l10n!(
            CATALOG,
            lang,
            "invite-server-error",
            message = "upstream 原文 / 503",
            kind = "pairing_closed_42"
        );
        let expected = wonton_l10n::l10n!(
            CATALOG,
            lang,
            "invite-request-failed",
            reason = upstream_detail
        );
        assert_eq!(reason, HomeFailure::UpstreamUnavailable);
        assert_eq!(detail, expected);
        assert_error_prefixes(lang);
    }
}

fn assert_error_prefixes(lang: wonton_l10n::Lang) {
    let raw = "path/to/store: upstream 原文 / 503";
    for (error, id) in [
        (
            CoreError::Storage { reason: raw.into() },
            "core-error-storage",
        ),
        (
            CoreError::NotReady { reason: raw.into() },
            "core-error-not-ready",
        ),
        (
            CoreError::TransportUnavailable { reason: raw.into() },
            "core-error-transport",
        ),
        (
            CoreError::Unauthorized { reason: raw.into() },
            "core-error-unauthorized",
        ),
        (
            CoreError::Forbidden { reason: raw.into() },
            "core-error-forbidden",
        ),
        (
            CoreError::Upstream { reason: raw.into() },
            "core-error-upstream",
        ),
    ] {
        assert_eq!(
            error.to_string(),
            wonton_l10n::l10n!(CATALOG, lang, id, reason = raw)
        );
    }
    assert_eq!(
        CoreError::InvalidInput { reason: raw.into() }.to_string(),
        raw
    );
}

async fn wait_home(
    subscription: &HomeSubscription,
    done: impl Fn(&HomeViewModel) -> bool,
) -> HomeViewModel {
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(view) = subscription.next().await {
            if done(&view) {
                return view;
            }
        }
        panic!("home subscription closed");
    })
    .await
    .expect("home state deadline")
}
