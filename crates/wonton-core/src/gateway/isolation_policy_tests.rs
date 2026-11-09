use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use hyper::StatusCode;
use tokio::net::TcpListener;

use super::tests::{CORE, Harness, PLUGIN, TOKEN, WHOAMI};
use crate::plugin::languages::Languages;
use crate::plugin::{PluginPackage, PluginStatus};

const POLICY: &str = "document-isolation-policy";
const ISOLATED: &str = "isolate-and-credentialless";

async fn upstream() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(|request: Request| async move {
        let mut response = Response::builder().header("content-type", "text/html");
        if request.uri().path() == "/own-policy" {
            response = response
                .header(POLICY, "none")
                .header(POLICY, "isolate-and-require-corp");
        }
        if request.uri().path() == "/failed" {
            response = response.status(StatusCode::SERVICE_UNAVAILABLE);
        }
        response.body(Body::from("<h1>Plugin</h1>")).unwrap()
    });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

fn register(h: &Harness, declared: Option<bool>, address: SocketAddr) {
    let source = match declared {
        Some(value) => WHOAMI.replace(
            "    port: 8000",
            &format!("    port: 8000\n    cross_origin_isolated: {value}"),
        ),
        None => WHOAMI.to_owned(),
    };
    h.manager.register_for_test(
        PluginPackage::manifest_of(&source, &Languages::from_header(Some("zh-Hans"))).unwrap(),
        PluginStatus::Running,
        Some(address.to_string()),
    );
}

#[tokio::test]
async fn isolation_policy_is_opt_in_and_replaces_all_upstream_values() {
    let h = Harness::start(None).await;
    let address = upstream().await;
    let session = h.app_session().await;
    for declared in [None, Some(false), Some(true)] {
        register(&h, declared, address);
        let page = h.get(PLUGIN, "/", &[("cookie", &session)]).await;
        assert_eq!(page.status, StatusCode::OK);
        assert_eq!(page.text(), "<h1>Plugin</h1>");
        assert_eq!(
            page.headers
                .get(POLICY)
                .map(|value| value.to_str().unwrap()),
            (declared == Some(true)).then_some(ISOLATED),
        );
        let page = h.get(PLUGIN, "/own-policy", &[("cookie", &session)]).await;
        assert_eq!(page.status, StatusCode::OK);
        let policies: Vec<_> = page.headers.get_all(POLICY).iter().collect();
        if declared == Some(true) {
            assert_eq!(policies, [ISOLATED]);
        } else {
            assert_eq!(policies, ["none", "isolate-and-require-corp"]);
        }
    }
    let error = h.get(PLUGIN, "/failed", &[("cookie", &session)]).await;
    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.headers[POLICY], ISOLATED);
}

#[tokio::test]
async fn core_pages_and_gateway_errors_do_not_get_plugin_isolation_policy() {
    let h = Harness::start(None).await;
    let address = upstream().await;
    register(&h, Some(true), address);
    for path in ["/", "/_wonton/login", "/_wonton/health"] {
        let page = h.get(CORE, path, &[]).await;
        assert!(!page.headers.contains_key(POLICY), "{path}");
    }
    let redirect = h.get(PLUGIN, "/", &[("accept", "text/html")]).await;
    assert_eq!(redirect.status, StatusCode::SEE_OTHER);
    assert!(!redirect.headers.contains_key(POLICY));
    let session = h.app_session().await;
    let stopped = TcpListener::bind("127.0.0.1:0").await.unwrap();
    register(&h, Some(true), stopped.local_addr().unwrap());
    drop(stopped);
    let error = h.get(PLUGIN, "/", &[("cookie", &session)]).await;
    assert_eq!(error.status, StatusCode::BAD_GATEWAY);
    assert_eq!(error.error_kind(), Some("upstream_unreachable"));
    assert!(!error.headers.contains_key(POLICY));
}

#[tokio::test]
async fn installed_isolation_notice_follows_request_language_and_declaration() {
    let h = Harness::start(None).await;
    let address = upstream().await;
    for declared in [None, Some(false), Some(true)] {
        register(&h, declared, address);
        let mut localized = Vec::new();
        for language in ["zh-Hans", "en-US"] {
            let reply = h
                .get(
                    CORE,
                    "/_wonton/api/plugins/app.whoami",
                    &[
                        ("authorization", &format!("Bearer {TOKEN}")),
                        ("accept-language", language),
                    ],
                )
                .await;
            assert_eq!(reply.status, StatusCode::OK);
            let detail = reply.json();
            let notices = detail["notices"].as_array().unwrap();
            if declared == Some(true) {
                let [notice] = notices.as_slice() else {
                    panic!("{detail}")
                };
                assert_eq!(notice["emphasis"], true);
                localized.push(notice["text"].as_str().unwrap().to_owned());
            } else {
                assert!(notices.is_empty(), "{detail}");
            }
        }
        if declared == Some(true) {
            assert_ne!(localized[0], localized[1]);
            assert!(!localized[0].is_ascii());
            assert!(localized[1].is_ascii());
        }
    }
}
