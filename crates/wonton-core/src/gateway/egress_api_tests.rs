//! 出网代理的管理接口（ADR-0023 的“接口：App 和核心”）：主人令牌鉴权，错误按类别给出 HTTP 状态，探测的原因放在 `detail`。

use std::sync::{Arc, Mutex};

use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use wonton_protocol::{EgressFailure, EgressProblem};

use super::tests::{CORE, Harness, Reply, TOKEN};
use crate::plugin::egress::address::ProxyAddress;
use crate::plugin::egress::probe::{Prober, Scripted};
use crate::plugin::egress::settings::Egress;
use crate::plugin::languages::Languages;

const PATH: &str = "/_wonton/api/egress-proxy";

async fn put(h: &Harness, body: &Value) -> Reply {
    let bearer = format!("Bearer {TOKEN}");
    let auth = [("authorization", bearer.as_str())];
    let body = Body::from(body.to_string());
    h.send(Method::PUT, CORE, PATH, &auth, body).await
}

async fn get(h: &Harness) -> Value {
    let bearer = format!("Bearer {TOKEN}");
    let reply = h.get(CORE, PATH, &[("authorization", &bearer)]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    reply.json()
}

fn update(url: Option<&str>, direct_cidrs: &[&str]) -> Value {
    json!({"url": url, "direct_cidrs": direct_cidrs, "restarts": []})
}

#[tokio::test]
async fn setting_and_cancelling_a_proxy_takes_effect_and_never_shows_the_password() {
    let h = Harness::start(None).await;
    assert_eq!(
        h.get(CORE, PATH, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let unset = get(&h).await;
    assert_eq!(unset["url"], Value::Null);
    assert_eq!(unset["source"], "none");

    let url = "http://me:s3cret@proxy.lan:7899";
    let saved = put(&h, &update(Some(url), &["100.64.0.0/10"])).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    assert!(!saved.text().contains("s3cret"), "{}", saved.text());
    let saved = saved.json();
    assert_eq!(saved["source"], "settings");
    assert_eq!(saved["direct_cidrs"], json!(["100.64.0.0/10"]));
    assert_eq!(saved["plugin_path"]["problem"], Value::Null);
    assert_eq!(saved["core_path"]["problem"], Value::Null);

    // App 拿不到密码，只改网段时带回打码的地址：密码不能变。
    let masked = saved["url"].as_str().unwrap();
    let kept = put(&h, &update(Some(masked), &[])).await;
    assert_eq!(kept.status, StatusCode::OK, "{}", kept.text());
    assert_eq!(kept.json()["direct_cidrs"], json!([]));

    let cancelled = put(&h, &update(None, &[])).await;
    assert_eq!(cancelled.status, StatusCode::OK, "{}", cancelled.text());
    assert_eq!(cancelled.json()["source"], "none");
}

#[tokio::test]
async fn bad_input_is_refused_by_kind_and_nothing_is_stored() {
    let h = Harness::start(None).await;
    let good = "http://proxy.lan:7899";
    for (body, status, kind) in [
        (
            update(Some("ftp://proxy.lan:21"), &[]),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (
            update(Some(good), &["not-a-cidr"]),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (
            json!({"url": good, "direct_cidrs": [], "restarts": ["app.gone"]}),
            StatusCode::CONFLICT,
            "changed",
        ),
        (
            json!({"url": good, "direct_cidrs": [], "restarts": [], "use_plugin": true}),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (
            json!({"url": null, "direct_cidrs": [], "restarts": [], "use_plugin": true}),
            StatusCode::NOT_FOUND,
            "not_installed",
        ),
    ] {
        let reply = put(&h, &body).await;
        assert_eq!(reply.status, status, "{}", reply.text());
        assert_eq!(reply.json()["error"]["kind"], kind);
        assert_eq!(get(&h).await["url"], Value::Null);
    }
}

#[tokio::test]
async fn an_address_set_at_install_cannot_be_changed_here_but_the_direct_list_can() {
    let env = ProxyAddress::parse("http://env.lan:7899", &Languages::default()).unwrap();
    let h = Harness::start_with(None, Egress::new(Some(env), None)).await;
    let reply = put(&h, &update(Some("http://other.lan:1"), &[])).await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text());
    assert_eq!(reply.json()["error"]["kind"], "conflict");

    let reply = put(&h, &update(Some("http://env.lan:7899"), &["100.64.0.0/10"])).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let reply = reply.json();
    assert_eq!(reply["source"], "env");
    assert_eq!(reply["direct_cidrs"], json!(["100.64.0.0/10"]));
}

#[tokio::test]
async fn a_proxy_that_does_not_answer_is_not_stored_and_the_reason_is_in_the_detail() {
    let script = Scripted {
        direct: Some(EgressProblem {
            reason: EgressFailure::ProxyUnreachable,
            message: "5 秒没响应".to_owned(),
        }),
        ..Scripted::default()
    };
    let prober = Prober::Fixed(Arc::new(Mutex::new(script)));
    let h = Harness::start_with(None, Egress::default().with_prober(prober)).await;
    let reply = put(&h, &update(Some("http://proxy.lan:7899"), &[])).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error = reply.json();
    assert_eq!(error["error"]["kind"], "egress_proxy_unavailable");
    assert_eq!(error["error"]["detail"], "proxy_unreachable");
    assert_eq!(error["error"]["message"], "5 秒没响应");
    assert_eq!(get(&h).await["url"], Value::Null);
}

fn proxy_plugin(id: &str) -> String {
    format!(
        "services:\n  proxy: {{image: nginx}}\nx-wonton:\n  spec: '0.5'\n  id: {id}\n  version: 1.0.0\n  kind: app\n  title: {{zh: 代理, en: Proxy}}\n  description: {{zh: 代理, en: Proxy}}\n  provides:\n    egress_proxy: {{service: proxy, port: 7890}}\n  permissions: [network.egress, egress.proxy]\n"
    )
}

async fn install_proxy(h: &Harness, id: &str) -> Reply {
    let bearer = format!("Bearer {TOKEN}");
    h.send(
        Method::POST,
        CORE,
        "/_wonton/api/plugins?grant=network.egress,egress.proxy",
        &[("authorization", &bearer)],
        Body::from(proxy_plugin(id)),
    )
    .await
}

#[tokio::test]
async fn egress_plugin_reports_provider_and_refuses_duplicate_and_active_uninstall() {
    let h = Harness::start(None).await;
    let installed = install_proxy(&h, "app.proxy").await;
    assert_eq!(
        installed.status,
        StatusCode::CREATED,
        "{}",
        installed.text()
    );
    let before = get(&h).await;
    assert_eq!(before["source"], "none");
    assert_eq!(before["provider"]["id"], "app.proxy");
    let selected = put(
        &h,
        &json!({"url": null, "direct_cidrs": [], "restarts": [], "use_plugin": true}),
    )
    .await;
    assert_eq!(selected.status, StatusCode::OK, "{}", selected.text());
    assert_eq!(selected.json()["source"], "plugin");
    assert_eq!(selected.json()["url"], Value::Null);
    assert_eq!(
        selected.json()["provider"],
        json!({"id": "app.proxy", "title": "Proxy"})
    );
    let duplicate = install_proxy(&h, "app.other").await;
    assert_eq!(
        duplicate.status,
        StatusCode::CONFLICT,
        "{}",
        duplicate.text()
    );
    assert_eq!(duplicate.json()["error"]["kind"], "conflict");
    let bearer = format!("Bearer {TOKEN}");
    let removed = h
        .send(
            Method::DELETE,
            CORE,
            "/_wonton/api/plugins/app.proxy",
            &[("authorization", &bearer)],
            Body::empty(),
        )
        .await;
    assert_eq!(removed.status, StatusCode::CONFLICT, "{}", removed.text());
    assert_eq!(removed.json()["error"]["kind"], "conflict");
}

#[tokio::test]
async fn egress_plugin_cannot_replace_the_environment_proxy() {
    let env = ProxyAddress::parse("http://env.lan:7899", &Languages::default()).unwrap();
    let h = Harness::start_with(None, Egress::new(Some(env), None)).await;
    assert_eq!(
        install_proxy(&h, "app.proxy").await.status,
        StatusCode::CREATED
    );
    let reply = put(
        &h,
        &json!({"url": null, "direct_cidrs": [], "restarts": [], "use_plugin": true}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text());
    assert_eq!(reply.json()["error"]["kind"], "conflict");
    assert_eq!(get(&h).await["source"], "env");
}
