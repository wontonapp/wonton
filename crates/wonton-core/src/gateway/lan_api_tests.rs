//! 局域网设置的管理接口（ADR-0019 的“接口：App 和核心”）：主人令牌鉴权，错误按类别给出 HTTP 状态。

use std::sync::{Arc, Mutex};

use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::json;

use super::tests::{CORE, Harness, TOKEN};
use crate::plugin::lan::probe::{DefaultRoute, HostAddress, HostInterface, HostNetwork};
use crate::plugin::lan::{Detector, Lan};

/// 测试网关用的宿主：有线 eth0 走默认路由；eth1 上只有 IPv6 的 ULA。
pub(super) fn lan() -> Lan {
    let nic = |name: &str, ip: &str| HostInterface {
        name: name.into(),
        physical: true,
        addresses: vec![HostAddress {
            ip: ip.parse().unwrap(),
            secondary: false,
        }],
        ..Default::default()
    };
    let network = HostNetwork {
        routes: vec![DefaultRoute {
            interface: "eth0".into(),
            metric: 100,
        }],
        interfaces: vec![nic("eth0", "192.168.1.10"), nic("eth1", "fd00::1")],
    };
    Lan::new(Detector::Fixed(Arc::new(Mutex::new(Ok(network)))), None)
}

#[tokio::test]
async fn lan_settings_need_the_owner_and_report_bad_choices_by_kind() {
    let h = Harness::start(None).await;
    assert_eq!(
        h.get(CORE, "/_wonton/api/lan", &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let bearer = format!("Bearer {TOKEN}");
    let auth = [("authorization", bearer.as_str())];
    let settings = h.get(CORE, "/_wonton/api/lan", &auth).await;
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.text());
    let settings = settings.json();
    assert_eq!(settings["choice"], json!({"mode": "auto"}));
    assert_eq!(settings["addresses"][0]["address"], "192.168.1.10");
    assert_eq!(settings["addresses"][0]["interface"], "eth0");

    let put = |body: &'static str| {
        h.send(
            Method::PUT,
            CORE,
            "/_wonton/api/lan",
            &auth,
            Body::from(body),
        )
    };
    for (body, status, kind) in [
        ("{", StatusCode::BAD_REQUEST, "invalid_input"),
        (
            r#"{"mode": "manual", "interfaces": [], "addresses": []}"#,
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (
            r#"{"mode": "manual", "interfaces": ["eth1"], "addresses": []}"#,
            StatusCode::UNPROCESSABLE_ENTITY,
            "lan_unavailable",
        ),
    ] {
        let reply = put(body).await;
        assert_eq!(reply.status, status, "{body}：{}", reply.text());
        assert_eq!(reply.error_kind(), Some(kind), "{body}");
    }
    let chosen =
        put(r#"{"mode": "manual", "interfaces": ["eth0"], "addresses": ["fd00::1"]}"#).await;
    assert_eq!(chosen.status, StatusCode::OK, "{}", chosen.text());
    let chosen = chosen.json();
    assert_eq!(chosen["choice"]["mode"], "manual");
    assert_eq!(chosen["addresses"][1]["address"], "fd00::1");
    assert_eq!(chosen["addresses"][1]["proxied"], true);
}
