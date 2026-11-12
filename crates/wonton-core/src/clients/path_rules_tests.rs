use super::path_rules::*;

fn deny(prefixes: &[&str]) -> PathRules {
    PathRules::Deny(prefixes.iter().map(|prefix| (*prefix).to_owned()).collect())
}

fn allow(prefixes: &[&str]) -> PathRules {
    PathRules::Allow(prefixes.iter().map(|prefix| (*prefix).to_owned()).collect())
}

fn failure(path: &str, status: &[u16]) -> AuthFailureRule {
    AuthFailureRule {
        path: path.to_owned(),
        status: status.to_vec(),
    }
}

#[test]
fn dot_segments_rejected() {
    // 0021-verification 的路径表：应用会各自解成点段、绕到前缀以外的写法一律 400。
    for raw in [
        "/x/y%2Fz/%2e%2e/%2e%2e/admin",
        "/admin/x//..//..",
        "/admin/..;/x",
        "/x/../admin",
        "/x%2f..%2fadmin",
        "/x/..\\admin",
        "/%252e%252e/admin",
    ] {
        assert_eq!(check_dot_segments(raw), Err(Invalid), "{raw}");
    }
}

#[test]
fn plain_paths_pass() {
    for raw in [
        "/",
        "/admin",
        "/share/x",
        "/user/repo/src/branch/feature%2Fx",
    ] {
        assert_eq!(check_dot_segments(raw), Ok(()), "{raw}");
    }
}

#[test]
fn deny_hits_every_candidate() {
    let rules = deny(&["/admin"]);
    for raw in [
        "/ADMIN",
        "//admin",
        "/%61dmin",
        "/admin;x",
        "/%2561dmin",
        "/admin/",
        "/admin/x",
    ] {
        assert!(!public_allows(&rules, raw), "{raw}");
    }
    for raw in ["/admin-x", "/x/admin", "/"] {
        assert!(public_allows(&rules, raw), "{raw}");
    }
}

#[test]
fn allow_requires_the_literal_path_too() {
    let rules = allow(&["/share"]);
    assert!(public_allows(&rules, "/share"));
    assert!(public_allows(&rules, "/share/x"));
    // 字面处理 `;` 的应用看到的是 `share;x`，不是声明的前缀。
    assert!(!public_allows(&rules, "/share;x"));
    assert!(!public_allows(&rules, "/sharex"));
}

#[test]
fn forgejo_encoded_branch_passes() {
    assert!(public_allows(
        &deny(&["/admin"]),
        "/user/repo/src/branch/feature%2Fx"
    ));
}

#[test]
fn invalid_percent_kept() {
    // 非法的百分号序列不报错，也不丢候选。
    assert_eq!(check_dot_segments("/a%zz"), Ok(()));
    assert!(public_allows(&deny(&["/admin"]), "/a%zz"));
    assert!(!public_allows(&deny(&["/a%zz"]), "/a%zz"));
}

#[test]
fn triple_decoding_is_not_covered() {
    // 解码三次才还原成 `/admin` 的写法不在防护范围（ADR-0021 写明的边界），第三份候选停在 `/%61dmin`。
    assert!(public_allows(&deny(&["/admin"]), "/%252561dmin"));
    assert_eq!(check_dot_segments("/%25252e%25252e/admin"), Ok(()));
}

#[test]
fn auth_failure_matches_candidates() {
    let rules = [failure("/user/login", &[401, 403])];
    assert!(is_auth_failure(&rules, "/USER/LOGIN", 401));
    assert!(is_auth_failure(&rules, "/user/login;x", 403));
    assert!(is_auth_failure(&rules, "/%75ser/login", 401));
    assert!(is_auth_failure(&rules, "/user/login/", 403));
    assert!(!is_auth_failure(&rules, "/user/login", 200));
    assert!(!is_auth_failure(&rules, "/user/logout", 401));
    assert!(!is_auth_failure(&rules, "/user/loginx", 401));
}

#[test]
fn header_names_normalize_underscore() {
    assert_eq!(normalize_header_name("X_Remote_User"), "x-remote-user");
    assert_eq!(normalize_header_name("X-Wonton-Entry"), "x-wonton-entry");
    assert!(same_header("X_Remote_User", "x-remote-user"));
    assert!(same_header("X-Forwarded-For", "x_forwarded_for"));
    assert!(!same_header("X-Remote-User", "X-Remote-Group"));
}

#[test]
fn prefix_slashes_trimmed() {
    assert!(public_allows(&allow(&["/share//"]), "/share/x"));
    assert!(public_allows(&allow(&["/"]), "/anything"));
    assert!(!public_allows(&deny(&["/"]), "/anything"));
    assert!(public_allows(&allow(&[""]), "/anything"));
}

#[test]
fn prefix_case_ignored() {
    assert!(!public_allows(&deny(&["/AdMiN"]), "/admin"));
    assert!(public_allows(&allow(&["/Share"]), "/share/x"));
    assert!(!public_allows(&allow(&["/Share"]), "/admin"));
}

#[test]
fn candidates_dedup_and_collapse() {
    assert_eq!(candidates("/ADMIN"), ["/admin"]);
    // 解码一次和两次相同，只留一份。
    assert_eq!(candidates("/%61dmin"), ["/%61dmin", "/admin"]);
    assert_eq!(candidates("\\share\\\\x"), ["/share/x"]);
    assert_eq!(candidates("/"), ["/"]);
}
