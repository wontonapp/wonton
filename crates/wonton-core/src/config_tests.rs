use super::*;

fn config(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect();
    Config::from_lookup(|name| {
        owned
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    })
}

#[test]
fn hosts_are_classified_by_subdomain() {
    let cfg = config(&[("WONTON_DATA_DIR", "/srv/wonton")]).unwrap();
    let urls = &cfg.public;
    assert_eq!(urls.classify("wonton.localhost:8080"), HostKind::Core);
    assert_eq!(urls.classify("AUTH.wonton.localhost"), HostKind::Auth);
    assert_eq!(
        urls.classify("matrix.wonton.localhost:8080"),
        HostKind::Matrix
    );
    assert_eq!(
        urls.classify("app-grafana.wonton.localhost:8080"),
        HostKind::Plugin("app-grafana".into())
    );
    assert_eq!(
        urls.classify("matrix.wonton.internal:8080"),
        HostKind::PluginMatrix
    );
    // 浏览器和插件各用一个主机名，都算身份提供方。
    assert_eq!(urls.classify("auth.wonton.internal:8080"), HostKind::Auth);
    assert_eq!(
        urls.classify("models.wonton.internal"),
        HostKind::PluginModels
    );
    assert_eq!(urls.classify("push.wonton.internal"), HostKind::Push);
    assert_eq!(
        urls.classify("files-hooks.wonton.internal:8080"),
        HostKind::FileHooks
    );
    assert_eq!(
        urls.classify("notify.wonton.internal"),
        HostKind::PluginNotify
    );
    for other in [
        "a.b.wonton.localhost",
        "evil.example",
        "wonton.localhost.evil",
        "mail.wonton.internal",
        "",
    ] {
        assert_eq!(urls.classify(other), HostKind::Unknown, "{other}");
    }
    assert_eq!(
        urls.origin("auth.wonton.localhost"),
        "http://auth.wonton.localhost:8080"
    );
    assert_eq!(
        urls.auth_internal_origin(),
        "http://auth.wonton.internal:8080"
    );
}

#[test]
fn a_proxy_that_is_not_an_address_stops_the_core_from_starting() {
    for name in ["WONTON_EGRESS_PROXY", "WONTON_EGRESS_PROXY_DETECTED"] {
        let with = |value| config(&[("WONTON_DATA_DIR", "/d"), (name, value)]);
        assert!(with("ftp://proxy:21").is_err(), "{name}");
        assert!(with("http://proxy:7899").is_ok(), "{name}");
    }
}

#[test]
fn a_doh_server_that_is_not_an_address_stops_the_core_from_starting() {
    let with = |value| config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_DOH_URL", value)]);
    assert!(with("ftp://1.1.1.1/dns-query").is_err());
    assert!(with("https://1.1.1.1").is_err());
    assert!(with("https://1.1.1.1/dns-query").is_ok());
}

#[test]
fn invalid_settings_fail_fast() {
    assert!(matches!(config(&[]), Err(ConfigError::Missing(_))));
    assert!(config(&[("WONTON_DATA_DIR", "relative")]).is_err());
    for domain in ["192.168.1.2", "wonton.internal", "home.wonton.internal"] {
        let settings = [("WONTON_DATA_DIR", "/d"), ("WONTON_BASE_DOMAIN", domain)];
        assert!(config(&settings).is_err(), "{domain}");
    }
    assert!(config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_PUBLIC_PORT", "x")]).is_err());
    assert!(config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_TUNNEL_PORT", "70000")]).is_err());
    assert!(config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_INSTANCE", "dev-1")]).is_err());
    // 局域网端口不能开在公网地址上：核心启动时就报错退出。
    let lan = |value| config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_LAN_ADDRESSES", value)]);
    assert!(lan("192.168.0.10,203.0.113.5").is_err());
    let addresses = lan("192.168.0.10, fd00::10")
        .unwrap()
        .lan_addresses
        .unwrap();
    assert_eq!(addresses.len(), 2);
    // 宣告地址同样只收私有地址，前缀可以带，超长不行。
    let announce = |value| {
        config(&[
            ("WONTON_DATA_DIR", "/d"),
            ("WONTON_ANNOUNCE_ADDRESSES", value),
        ])
    };
    assert!(announce("192.168.1.20,203.0.113.5").is_err());
    assert!(announce("192.168.1.20/40").is_err());
    let announced = announce("192.168.1.20/24")
        .unwrap()
        .announce_addresses
        .unwrap();
    assert_eq!(announced[0].ip.to_string(), "192.168.1.20");
    assert_eq!(announced[0].prefix, Some(24));
    assert!(
        config(&[
            ("WONTON_DATA_DIR", "/d"),
            ("WONTON_OIDC_ADMIN_URL", "http://h:4445")
        ])
        .is_err()
    );
}

#[test]
fn oidc_provider_urls_must_be_plain_http_origins() {
    let with_public = |public_url: &str| {
        config(&[
            ("WONTON_DATA_DIR", "/d"),
            ("WONTON_OIDC_ADMIN_URL", "http://hydra:4445/"),
            ("WONTON_OIDC_PUBLIC_URL", public_url),
        ])
    };
    let oidc = with_public("HTTP://Hydra:4444/").unwrap().oidc.unwrap();
    assert_eq!(oidc.admin_url, "http://hydra:4445");
    assert_eq!(oidc.public_url, "http://hydra:4444");
    let oidc = with_public("http://[::1]").unwrap().oidc.unwrap();
    assert_eq!(oidc.public_url, "http://[::1]:80");
    for bad in [
        "https://hydra:4444",
        "http://hydra:4444/oidc",
        "http://hydra:4444/?x=1",
        "http://hydra:4444#top",
        "http://user:secret@hydra:4444",
        "hydra:4444",
        "http://",
    ] {
        let error = with_public(bad).unwrap_err();
        assert!(
            matches!(
                &error,
                ConfigError::Invalid {
                    name: "WONTON_OIDC_PUBLIC_URL",
                    ..
                }
            ),
            "{bad}：{error}"
        );
        assert!(!error.to_string().contains("secret"), "{error}");
    }
}

#[test]
fn the_owner_name_is_checked_with_or_without_matrix() {
    let matrix = [
        ("WONTON_DATA_DIR", "/d"),
        ("WONTON_MATRIX_URL", "http://tuwunel:8008"),
        ("WONTON_MATRIX_JWT_KEY", "-----BEGIN PRIVATE KEY-----"),
        ("WONTON_MATRIX_REGISTRATION_SECRET", "s3cret"),
    ];
    for base in [&matrix[..], &matrix[..1]] {
        let with_owner = |name: &'static str| {
            let mut settings = base.to_vec();
            settings.push(("WONTON_OWNER_NAME", name));
            config(&settings)
        };
        assert_eq!(with_owner("alice-1").unwrap().username, "alice-1");
        for bad in ["Alice", "a/b", "a+b", "a=b", "_foo"] {
            let error = with_owner(bad).unwrap_err().to_string();
            assert!(error.contains("WONTON_OWNER_NAME"), "{bad}：{error}");
        }
    }
}

#[test]
fn the_owner_name_is_locked_against_the_record_in_the_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_str().unwrap();
    let start = |name: Option<&'static str>| {
        let mut settings = vec![("WONTON_DATA_DIR", data)];
        settings.extend(name.map(|name| ("WONTON_OWNER_NAME", name)));
        config(&settings).unwrap().lock_owner()
    };
    assert_eq!(start(Some("alice")).unwrap().username, "alice");
    assert_eq!(start(None).unwrap().username, "alice");
    assert!(start(Some("bob")).is_err());
}

#[test]
fn the_time_zone_must_be_an_iana_name() {
    let base = ("WONTON_DATA_DIR", "/d");
    assert_eq!(config(&[base]).unwrap().tz, "UTC");
    assert_eq!(
        config(&[base, ("TZ", "Asia/Shanghai")]).unwrap().tz,
        "Asia/Shanghai"
    );
    for bad in ["CST-8", ":/etc/localtime"] {
        let error = config(&[base, ("TZ", bad)]).unwrap_err().to_string();
        assert!(error.contains("TZ"), "{bad}：{error}");
    }
}

#[test]
fn matrix_secrets_can_come_from_files_but_not_from_both_places() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("matrix-jwt.pem");
    let secret = dir.path().join("matrix-registration-secret");
    std::fs::write(&key, "-----BEGIN PRIVATE KEY-----\n").unwrap();
    std::fs::write(&secret, "s3cret\n").unwrap();
    let (key, secret) = (key.to_str().unwrap(), secret.to_str().unwrap());
    let files = [
        ("WONTON_DATA_DIR", "/d"),
        ("WONTON_MATRIX_URL", "http://tuwunel:8008"),
        ("WONTON_MATRIX_JWT_KEY_FILE", key),
        ("WONTON_MATRIX_REGISTRATION_SECRET_FILE", secret),
    ];
    let matrix = config(&files).unwrap().matrix.unwrap();
    assert_eq!(matrix.registration_secret, "s3cret");
    assert_eq!(
        matrix.server_name, "wonton.localhost",
        "没记下名字时是基础域名"
    );

    let name = dir.path().join("matrix-server-name");
    std::fs::write(&name, "abc.wonton.internal\n").unwrap();
    let mut named = files.to_vec();
    named.push(("WONTON_MATRIX_SERVER_NAME_FILE", name.to_str().unwrap()));
    let matrix = config(&named).unwrap().matrix.unwrap();
    assert_eq!(matrix.server_name, "abc.wonton.internal");
    assert!(matrix.jwt_key.starts_with("-----BEGIN PRIVATE KEY-----"));

    let mut both = files.to_vec();
    both.push(("WONTON_MATRIX_REGISTRATION_SECRET", "other"));
    assert!(config(&both).is_err(), "值和文件只能给一个");
    let mut missing = files.to_vec();
    missing[2] = ("WONTON_MATRIX_JWT_KEY_FILE", "/no/such/file");
    assert!(config(&missing).is_err(), "文件读不了时启动失败");
}

#[test]
fn matrix_needs_its_url_key_and_secret_together() {
    let base = [("WONTON_DATA_DIR", "/d")];
    assert!(config(&base).unwrap().matrix.is_none());
    let full = [
        ("WONTON_DATA_DIR", "/d"),
        ("WONTON_MATRIX_URL", "http://tuwunel:8008/"),
        ("WONTON_MATRIX_JWT_KEY", "-----BEGIN PRIVATE KEY-----"),
        ("WONTON_MATRIX_REGISTRATION_SECRET", "s3cret"),
    ];
    let matrix = config(&full).unwrap().matrix.unwrap();
    assert_eq!(matrix.url, "http://tuwunel:8008");
    assert_eq!(matrix.server_name, "wonton.localhost");
    assert!(!format!("{matrix:?}").contains("s3cret"), "密钥不进日志");
    for missing in [1, 2, 3] {
        let partial: Vec<(&str, &str)> = full
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != missing)
            .map(|(_, pair)| *pair)
            .collect();
        assert!(config(&partial).is_err(), "少了 {}", full[missing].0);
    }
}

#[test]
fn plugin_network_pool_is_validated_before_startup() {
    for value in ["10.213.0.0/27", "203.0.113.0/24", "10.213.0.1/16"] {
        assert!(config(&[("WONTON_DATA_DIR", "/d"), (network_pool::VAR, value)]).is_err());
    }
    let cfg = config(&[
        ("WONTON_DATA_DIR", "/d"),
        (network_pool::VAR, "10.214.0.0/16"),
    ])
    .unwrap();
    assert_eq!(
        cfg.plugin_network_pool.subnet().to_string(),
        "10.214.0.0/16"
    );
}
