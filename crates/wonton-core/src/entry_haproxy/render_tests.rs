//! 渲染结果的关键行长什么样，以及不进配置的值一律报错。

use super::model::{AUTH_FAILURE_HEADER, Model, Trust};
use super::render::render;

fn model() -> Model {
    let mut model = Model::federation_only("home.example.test", "cloud.example.test");
    model.public_labels = vec!["forgejo".to_owned(), "immich".to_owned()];
    model.lan_labels = vec!["forgejo".to_owned()];
    model.lan_forward = Some("10.90.1.2".to_owned());
    model
}

fn frontend<'a>(config: &'a str, name: &str) -> &'a str {
    let start = config
        .find(&format!("frontend {name}\n"))
        .unwrap_or_else(|| panic!("没有 {name} 前端"));
    let end = config[start + 1..]
        .find("\nfrontend ")
        .map_or(config.len(), |offset| start + 1 + offset);
    &config[start..end]
}

#[test]
fn renders_only_open_labels_per_frontend() {
    let rendered = render(&model()).unwrap();
    let public: Vec<&str> = rendered.public_crt_list.split_whitespace().collect();
    let lan: Vec<&str> = rendered.lan_crt_list.split_whitespace().collect();
    assert_eq!(public[0], "/certs/site.pem");
    assert!(public.contains(&"forgejo.home.example.test"));
    assert!(public.contains(&"immich.home.example.test"));
    assert!(public.contains(&"cloud.example.test"));
    assert!(
        !public.iter().any(|name| name.contains(".lan.")),
        "{public:?}"
    );
    assert_eq!(lan, ["/certs/site.pem", "forgejo.lan.home.example.test"]);
    assert!(
        !lan[1..].iter().any(|name| !name.contains(".lan.")),
        "{lan:?}"
    );
}

#[test]
fn both_lists_use_the_same_certificate_file() {
    let rendered = render(&model()).unwrap();
    for list in [&rendered.public_crt_list, &rendered.lan_crt_list] {
        assert!(list.starts_with("/certs/site.pem "), "{list:?}");
        assert_eq!(list.lines().count(), 1, "{list:?}");
    }
}

#[test]
fn public_and_lan_trust_one_peer_each() {
    let rendered = render(&model()).unwrap();
    for (frontend, peer) in [
        (frontend(&rendered.config, "public"), "10.90.0.2"),
        (frontend(&rendered.config, "lan"), "10.90.1.2"),
    ] {
        assert!(frontend.contains(&format!("reject unless {{ src {peer} }}")));
        assert!(frontend.contains(&format!("expect-proxy layer4 if {{ src {peer} }}")));
        assert!(!frontend.contains("accept-proxy"));
        for other in ["10.90.0.2", "10.90.1.2"] {
            if other != peer {
                assert!(!frontend.contains(other), "{frontend}");
            }
        }
    }
}

/// `expect-proxy` 解析完 PROXY 头以后 `src` 就是客户端地址了；连接器那一侧的判断必须写在它前面。
#[test]
fn connector_is_checked_before_the_proxy_header_is_parsed() {
    let rendered = render(&model()).unwrap();
    let public = frontend(&rendered.config, "public");
    let reject = public.find("reject unless").expect("没有对端检查");
    let expect = public.find("expect-proxy").expect("没有 expect-proxy");
    assert!(reject < expect, "{public}");
}

#[test]
fn header_trust_reads_the_header_only_from_the_connector() {
    let mut model = model();
    model.trust = Trust::Header("CF-Connecting-IP".to_owned());
    let rendered = render(&model).unwrap();
    let public = frontend(&rendered.config, "public");
    assert!(!public.contains("expect-proxy"), "{public}");
    assert!(public.contains("reject unless { src 10.90.0.2 }"));
    assert!(public.contains("http-request set-src hdr(CF-Connecting-IP) if { src 10.90.0.2 }"));
    // 局域网一侧永远是 PROXY v2。
    assert!(frontend(&rendered.config, "lan").contains("expect-proxy"));
}

#[test]
fn strips_forwarded_and_wonton_headers_on_both_frontends() {
    let rendered = render(&model()).unwrap();
    for name in ["public", "lan"] {
        let frontend = frontend(&rendered.config, name);
        for header in [
            "Forwarded",
            "X-Forwarded-For",
            "X-Forwarded-Host",
            "X-Forwarded-Proto",
            "X-Forwarded-Port",
            "X-Forwarded-Prefix",
            "X-Real-IP",
            "X-Wonton-Entry",
            AUTH_FAILURE_HEADER,
        ] {
            assert!(
                frontend.contains(&format!("http-request del-header {header}\n")),
                "{name} 没删 {header}"
            );
        }
        assert!(frontend.contains("http-request set-header X-Real-IP %[src]"));
        assert!(frontend.contains(&format!("http-request set-header X-Wonton-Entry {name}")));
    }
}

#[test]
fn host_must_match_sni() {
    let rendered = render(&model()).unwrap();
    for name in ["public", "lan"] {
        let frontend = frontend(&rendered.config, name);
        assert!(frontend.contains("http-request set-var(txn.sni) ssl_fc_sni,lower"));
        assert!(frontend.contains(
            "http-request deny deny_status 421 unless { hdr(host),field(1,:),lower,strcmp(txn.sni) eq 0 }"
        ));
    }
}

#[test]
fn auth_failure_header_is_counted_and_removed() {
    let rendered = render(&model()).unwrap();
    for name in ["public", "lan"] {
        let frontend = frontend(&rendered.config, name);
        assert!(frontend.contains("acl src_v4 src 0.0.0.0/0"));
        assert!(frontend.contains("acl src_v6 src ::/0"));
        assert!(frontend.contains("http-request track-sc0 src table auth_failures4 if src_v4"));
        assert!(
            frontend.contains(
                "http-request track-sc1 src,ipmask(32,64) table auth_failures6 if src_v6"
            )
        );
        assert!(frontend.contains("acl banned_v4 sc_get_gpc0(0) gt 10"));
        assert!(frontend.contains("acl banned_v6 sc_get_gpc0(1) gt 10"));
        assert!(frontend.contains("http-request deny deny_status 429 if banned_v4 or banned_v6"));
        assert!(frontend.contains(&format!(
            "http-response sc-inc-gpc0(0) if {{ res.hdr({AUTH_FAILURE_HEADER}) -m found }}"
        )));
        assert!(frontend.contains(&format!(
            "http-response sc-inc-gpc0(1) if {{ res.hdr({AUTH_FAILURE_HEADER}) -m found }}"
        )));
        assert!(frontend.contains(&format!("http-response del-header {AUTH_FAILURE_HEADER}")));
    }
    // IPv4 单个地址，IPv6 /64：两张表。
    assert!(rendered.config.contains("stick-table type ip size"));
    assert!(rendered.config.contains("stick-table type ipv6 size"));
    assert!(rendered.config.contains("expire 15m store gpc0"));
}

#[test]
fn runtime_socket_binds_only_on_the_clients_network() {
    let rendered = render(&model()).unwrap();
    assert!(
        rendered
            .config
            .contains("stats socket entry.wonton.internal:9999 level operator")
    );
    let global = &rendered.config[..rendered.config.find("defaults").unwrap()];
    assert!(!global.contains("10.90.0.2"));
    assert!(!global.contains("10.90.1.2"));
}

#[test]
fn lan_frontend_is_absent_without_the_lan_forward_network() {
    let mut model = model();
    model.lan_forward = None;
    let rendered = render(&model).unwrap();
    assert!(!rendered.config.contains("frontend lan\n"));
    // 列表照样渲染，C8 建网络以后写同一份文件。
    assert!(
        rendered
            .lan_crt_list
            .contains("forgejo.lan.home.example.test")
    );
}

#[test]
fn federation_only_model_matches_the_previous_configuration() {
    let rendered = render(&Model::federation_only("home.example", "home.example")).unwrap();
    for line in [
        "frontend public\n  bind entry-edge.wonton.internal:8443 ssl crt-list /certs/public.crt-list strict-sni expose-fd listeners\n",
        "  tcp-request connection reject unless { src 10.90.0.2 }\n  tcp-request connection expect-proxy layer4 if { src 10.90.0.2 }\n",
        "  http-request deny deny_status 421 unless { hdr(host),field(1,:),lower,strcmp(txn.sni) eq 0 }\n",
        "  http-request del-header Forwarded\n  http-request del-header X-Forwarded-For\n  http-request del-header X-Forwarded-Host\n  http-request del-header X-Forwarded-Proto\n",
        "  http-request del-header X-Real-IP\n  http-request del-header X-Wonton-Entry\n",
        "  http-request set-header X-Real-IP %[src]\n  http-request set-header X-Wonton-Entry public\n",
        "  default_backend gateway\n",
        "frontend paired\n  bind entry.wonton.internal:8449 ssl crt /certs/site.pem strict-sni expose-fd listeners\n  acl correct_sni ssl_fc_sni -i home.example\n  http-request deny deny_status 421 unless correct_sni\n  http-request deny deny_status 421 unless { hdr(host),lower -i home.example home.example:443 home.example:8448 }\n  http-request set-header Host home.example\n  default_backend gateway\n",
        "backend gateway\n  server core entry-gateway.wonton.internal:8484 resolvers docker init-addr libc,none\n",
        "resolvers docker\n  nameserver docker 127.0.0.11:53\n  resolve_retries 1\n  timeout resolve 1s\n  timeout retry 1s\n  hold valid 10s\n",
        "option http-restrict-req-hdr-names delete\n",
    ] {
        assert!(rendered.config.contains(line), "缺少 {line:?}");
    }
    assert_eq!(
        rendered.public_crt_list,
        "/certs/site.pem home.example probe.home.example\n"
    );
    assert!(rendered.lan_crt_list.is_empty());
}

#[test]
fn without_federation_only_the_probe_name_stays_in_the_public_list() {
    let mut model = Model::federation_only("home.example", "home.example");
    model.federation = None;
    model.paired_federation = false;
    let rendered = render(&model).unwrap();
    assert_eq!(
        rendered.public_crt_list,
        "/certs/site.pem probe.home.example\n"
    );
    assert!(rendered.lan_crt_list.is_empty());
    assert!(!rendered.config.contains("frontend paired"));
}

#[test]
fn config_injection_in_every_value_is_rejected() {
    let bad = [
        "im\nmich", "immich 2", "im#mich", "im{0}", "im%[src]", "im;mich", "-immich", "immich-", "",
    ];
    for value in bad {
        let mut model = model();
        model.public_labels = vec![value.to_owned()];
        assert!(render(&model).is_err(), "标签 {value:?} 该被拒");
    }
    for value in [
        "home example",
        "home.example{",
        "home.example\n",
        "home.",
        "home..example",
    ] {
        let mut model = model();
        model.suffix = value.to_owned();
        assert!(render(&model).is_err(), "后缀 {value:?} 该被拒");
    }
    for value in ["CF Connecting", "CF_Connecting", "1CF-Connecting", "CF{0}"] {
        let mut model = model();
        model.trust = Trust::Header(value.to_owned());
        assert!(render(&model).is_err(), "头名 {value:?} 该被拒");
    }
    for value in ["10.90.0.2x", "10.90.0.2\n", "connector.wonton.internal"] {
        let mut model = model();
        model.connectors = vec![value.to_owned()];
        assert!(render(&model).is_err(), "对端 {value:?} 该被拒");
    }
}

#[test]
fn duplicate_and_reserved_labels_are_rejected() {
    let mut model = self::model();
    model.public_labels = vec!["immich".to_owned(), "immich".to_owned()];
    assert!(render(&model).is_err());
    let mut model = self::model();
    model.lan_labels = vec!["lan".to_owned()];
    assert!(render(&model).is_err());
    let mut model = self::model();
    model.public_labels = vec!["probe".to_owned()];
    assert!(render(&model).is_err());
}
