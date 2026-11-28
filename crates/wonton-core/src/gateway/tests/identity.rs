//! 认证主机转发给身份提供方，发现文档按访问的名字重写。

use std::sync::Mutex;

use serde_json::json;

use super::super::fakes::fake_hydra;
use super::*;

#[tokio::test]
async fn auth_host_is_forwarded_to_the_identity_provider() {
    let hydra = fake_hydra(Arc::new(Mutex::new(Vec::new()))).await;
    let h = Harness::start(Some(hydra)).await;
    let path = "/oauth2/auth?x=1";
    let reply = h.get("auth.wonton.localhost", path, &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["path"], path);
}

#[tokio::test]
async fn discovery_endpoints_are_rewritten_by_who_reaches_them_under_both_names() {
    let hydra = fake_hydra(Arc::new(Mutex::new(Vec::new()))).await;
    let h = Harness::start(Some(hydra)).await;
    let port = h.addr.port();
    for host in ["auth.wonton.localhost", "auth.wonton.internal"] {
        let reply = h.get(host, "/.well-known/openid-configuration", &[]).await;
        assert_eq!(reply.status, StatusCode::OK, "{host}");
        let browser = format!("http://auth.wonton.localhost:{port}");
        let backend = format!("http://auth.wonton.internal:{port}");
        assert_eq!(
            reply.json(),
            json!({
                "issuer": "http://auth.wonton.internal",
                "authorization_endpoint": format!("{browser}/oauth2/auth"),
                "token_endpoint": format!("{backend}/oauth2/token"),
                "userinfo_endpoint": format!("{backend}/userinfo"),
                "jwks_uri": format!("{backend}/.well-known/jwks.json"),
                "revocation_endpoint": format!("{backend}/oauth2/revoke"),
                "end_session_endpoint": format!("{browser}/oauth2/sessions/logout"),
                "scopes_supported": ["openid"],
            }),
            "{host}"
        );
    }
}
