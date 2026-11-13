use super::*;

#[tokio::test]
async fn response_cookies_cannot_target_sibling_plugins_or_reserved_names() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    home.register(HOOK_APP, "hookapp", true, true);
    for (host, headers) in [
        (home.host("entryapp"), entry_public()),
        (home.lan_host("entryapp"), entry_lan()),
    ] {
        let reply = home.get(&host, "/cookies", &headers).await;
        assert_eq!(reply.status, StatusCode::OK);
        let cookies: Vec<_> = reply
            .headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert_eq!(
            cookies,
            ["session=attacker; Path=/", "theme=dark; Path=/; HttpOnly"],
            "{host}: cookies must remain host-only and cannot overwrite gateway cookies"
        );
    }
    let sibling = home
        .get(&home.host("hookapp"), "/webhook", &entry_public())
        .await;
    assert_eq!(sibling.status, StatusCode::OK);
    assert!(home.upstream_hits().last().unwrap().cookie().is_empty());
}

#[tokio::test]
async fn repeated_separator_rules_enforce_deny_and_auth_failures() {
    let home = Home::start().await;
    let compose = ENTRY_APP
        .replace("deny: [/admin]", "deny: [/api//admin]")
        .replace("path: /auth/login", "path: /auth//login");
    home.register(&compose, "entryapp", false, true);
    let host = home.host("entryapp");
    for path in ["/api/admin", "/api//admin", "/api/admin/child"] {
        let reply = home.get(&host, path, &entry_public()).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{path}");
    }
    assert!(home.upstream_hits().is_empty());
    for path in ["/auth/login", "/auth//login", "/auth/login/child"] {
        let reply = home.get(&host, path, &entry_public()).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(reply.headers.get("x-wonton-auth-failure").unwrap(), "1");
    }
    let near = home.get(&host, "/api/admin-extra", &entry_public()).await;
    assert_eq!(near.status, StatusCode::OK);
    let near = home.get(&host, "/auth/login-extra", &entry_public()).await;
    assert_eq!(near.status, StatusCode::UNAUTHORIZED);
    assert!(!near.headers.contains_key("x-wonton-auth-failure"));
}

#[tokio::test]
async fn reserved_namespace_variants_never_reach_plugins_on_either_entry() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    for (host, headers) in [
        (home.host("entryapp"), entry_public()),
        (home.lan_host("entryapp"), entry_lan()),
    ] {
        for path in [
            "/_wonton",
            "/%5fwonton/guest/x",
            "/%255fwonton/guest/x",
            "//_wonton/guest/x",
            "/_WONTON/guest/x",
            "/_wonton;session/guest/x",
            "/a/../_wonton/guest/x",
            "/a/%252e%252e/_wonton/guest/x",
        ] {
            let reply = home.get(&host, path, &headers).await;
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{host}{path}");
        }
        let ordinary = home.get(&host, "/_wonton-extra", &headers).await;
        assert_eq!(ordinary.status, StatusCode::OK);
    }
    assert!(
        home.upstream_hits()
            .iter()
            .all(|hit| hit.uri == "/_wonton-extra")
    );
}
