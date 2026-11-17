//! 清单里的公网规则：`public` 只在公网入口拒绝、`allow` 只放行列表内的路径；
//! 认证失败计数只记声明过的路径与状态码，冒充的计数头不算。

use super::*;

#[tokio::test]
async fn public_deny_and_allow_enforced_on_public_only() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    home.register(HOOK_APP, "hook", true, true);
    let public = home.host("entryapp");
    let lan = home.lan_host("entryapp");
    for path in ["/admin", "/admin/panel", "/ADMIN"] {
        let denied = home.get(&public, path, &entry_public()).await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(denied.error_kind(), Some("public_denied"));
    }
    // 同一条路径在局域网入口不受 `public` 限制。
    let lan_ok = home.get(&lan, "/admin", &entry_lan()).await;
    assert_eq!(lan_ok.status, StatusCode::OK);
    let allowed = home.get(&public, "/public", &entry_public()).await;
    assert_eq!(allowed.status, StatusCode::OK);
    // `allow` 反过来：列表之外的路径公网进不来，局域网照常。
    let hook_public = home.host("hook");
    let hook_lan = home.lan_host("hook");
    let ok = home.get(&hook_public, "/webhook", &entry_public()).await;
    assert_eq!(ok.status, StatusCode::OK);
    let blocked = home.get(&hook_public, "/other", &entry_public()).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    let lan_ok = home.get(&hook_lan, "/other", &entry_lan()).await;
    assert_eq!(lan_ok.status, StatusCode::OK);
    // 到后端的正好是放行的四条；被拒的三条一条都没到。
    let hits = home.upstream_hits();
    let uris: Vec<&str> = hits.iter().map(|hit| hit.uri.as_str()).collect();
    assert_eq!(uris, vec!["/admin", "/public", "/webhook", "/other"]);
}

#[tokio::test]
async fn auth_failure_marked_only_for_declared_paths() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let headers: Vec<(&str, &str)> = entry_public().to_vec();
    let marked = home.get(&host, "/auth/login", &headers).await;
    assert_eq!(marked.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        marked
            .headers
            .get("x-wonton-auth-failure")
            .map(|v| v.to_str().unwrap()),
        Some("1")
    );
    // 声明了路径但状态码不在列表里、或路径没声明：都不算验证失败。
    let undeclared_status = home.get(&host, "/auth/deny", &headers).await;
    assert_eq!(undeclared_status.status, StatusCode::UNAUTHORIZED);
    assert!(
        undeclared_status
            .headers
            .get("x-wonton-auth-failure")
            .is_none()
    );
    let undeclared_path = home.get(&host, "/auth/other", &headers).await;
    assert_eq!(undeclared_path.status, StatusCode::UNAUTHORIZED);
    assert!(
        undeclared_path
            .headers
            .get("x-wonton-auth-failure")
            .is_none()
    );
    // 插件自己带的计数头先删掉，冒充不了计数。
    let spoofed = home.get(&host, "/spoof", &headers).await;
    assert_eq!(spoofed.status, StatusCode::OK);
    assert!(spoofed.headers.get("x-wonton-auth-failure").is_none());
}
