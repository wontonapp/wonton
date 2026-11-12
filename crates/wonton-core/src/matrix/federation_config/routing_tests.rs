use std::collections::BTreeSet;

use regex::RegexSet;

use super::{PROXY_ADDR, config_toml};

fn config(server_name: &str, paired: &[&str], forbidden: &[&str]) -> toml::Value {
    let paired = paired.iter().map(|name| (*name).to_owned()).collect();
    let forbidden = forbidden.iter().map(|name| (*name).to_owned()).collect();
    toml::from_str(&config_toml(server_name, &paired, &forbidden)).unwrap()
}

fn strings(value: &toml::Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect()
}

fn assert_proxy(config: &toml::Value, included: &[&str]) {
    let rules = config["global"]["proxy"]["by_domain"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
    let rule = &rules[0];
    assert_eq!(
        rule["url"].as_str(),
        Some(format!("http://{PROXY_ADDR}").as_str())
    );
    let actual: BTreeSet<_> = strings(&rule["include"]).into_iter().collect();
    let expected: BTreeSet<_> = included.iter().copied().collect();
    assert_eq!(actual, expected);
    assert_eq!(strings(&rule["exclude"]), ["push.wonton.internal"]);
}

#[test]
fn named_server_without_pairings_disables_proxy_without_closing_federation() {
    let config = config("home.example.test", &[], &[]);
    let global = &config["global"];
    assert_eq!(global["proxy"].as_str(), Some("none"));
    assert!(
        global
            .get("allowed_remote_server_names_experimental")
            .is_none()
    );
    assert!(strings(&global["forbidden_remote_server_names"]).is_empty());
}

#[test]
fn named_server_routes_only_exact_paired_domains_through_the_proxy() {
    let peers = ["friend.example.test", "private.wonton.internal"];
    let config = config("home.example.test", &peers, &[]);
    assert_proxy(&config, &peers);
    assert!(
        config["global"]
            .get("allowed_remote_server_names_experimental")
            .is_none()
    );
    let included = strings(&config["global"]["proxy"]["by_domain"][0]["include"]);
    for other in [
        "matrix.org",
        "example.test",
        "sub.friend.example.test",
        "xxfriend.example.test",
        "friend.example.test.attacker.test",
    ] {
        assert!(!included.contains(&other), "{other}");
    }
}

#[test]
fn named_server_keeps_exact_domain_blocks_when_pairings_change() {
    let denied = ["blocked.example.test", "bad-server.example.test"];
    let phases: &[&[&str]] = &[
        &[],
        &["first.example.test"],
        &["first.example.test", "second.example.test"],
        &["second.example.test"],
        &[],
    ];
    for peers in phases {
        let config = config("home.example.test", peers, &denied);
        let global = &config["global"];
        assert!(
            global
                .get("allowed_remote_server_names_experimental")
                .is_none()
        );
        if peers.is_empty() {
            assert_eq!(global["proxy"].as_str(), Some("none"));
        } else {
            assert_proxy(&config, peers);
        }
        let blocked = RegexSet::new(strings(&global["forbidden_remote_server_names"])).unwrap();
        for domain in denied {
            assert!(blocked.is_match(domain), "{domain}");
        }
        for other in [
            "blockedXexampleXtest",
            "bad-serverXexampleXtest",
            "xxblocked.example.test",
            "sub.blocked.example.test",
            "blocked.example.test.attacker.test",
            "first.example.test",
            "second.example.test",
            "matrix.org",
        ] {
            assert!(!blocked.is_match(other), "{other}");
        }
    }
}

#[test]
fn private_server_without_pairings_keeps_the_closed_allowlist_and_proxy() {
    let config = config("home.wonton.internal", &[], &[]);
    assert_proxy(&config, &[]);
    let allowed = RegexSet::new(strings(
        &config["global"]["allowed_remote_server_names_experimental"],
    ))
    .unwrap();
    assert!(!allowed.is_empty(), "空正则集合会让私用服务器开放联邦");
    for other in [
        "friend.wonton.internal",
        "home.example.test",
        "matrix.org",
        "push.wonton.internal",
    ] {
        assert!(!allowed.is_match(other), "{other}");
    }
}

#[test]
fn private_server_updates_exact_allowlist_without_losing_domain_blocks() {
    let denied = ["blocked.example.test"];
    let phases: &[&[&str]] = &[
        &["first.wonton.internal"],
        &["first.wonton.internal", "second.example.test"],
        &["second.example.test"],
        &[],
    ];
    for peers in phases {
        let config = config("home.wonton.internal", peers, &denied);
        assert_proxy(&config, &[]);
        let global = &config["global"];
        let allowed =
            RegexSet::new(strings(&global["allowed_remote_server_names_experimental"])).unwrap();
        for candidate in ["first.wonton.internal", "second.example.test"] {
            assert_eq!(allowed.is_match(candidate), peers.contains(&candidate));
        }
        for other in [
            "xxfirst.wonton.internal",
            "sub.first.wonton.internal",
            "firstXwontonXinternal",
            "first.wonton.internal.attacker.test",
            "secondXexampleXtest",
            "second.example.test.attacker.test",
            "matrix.org",
        ] {
            assert!(!allowed.is_match(other), "{other}");
        }
        let blocked = RegexSet::new(strings(&global["forbidden_remote_server_names"])).unwrap();
        assert!(blocked.is_match(denied[0]));
        assert!(!blocked.is_match("blockedXexampleXtest"));
        assert!(!blocked.is_match("xxblocked.example.test"));
        assert!(!blocked.is_match("blocked.example.test.attacker.test"));
    }
}
