use super::*;
use std::path::Path;

fn relay(url: &str, tier: u8) -> Relay {
    Relay {
        url: url.parse().unwrap(),
        token: None,
        quic_port: None,
        tier,
    }
}

fn startup(builtin: Vec<Relay>, env: Option<&str>, file: Option<&Path>) -> Startup {
    Startup {
        builtin: RelayTable::from_relays(builtin),
        signed_url: None,
        signed_pubkey: None,
        env: env.map(|text| OwnerLayer::parse(text).unwrap()),
        file: file.map(Path::to_path_buf),
        proxy: None,
    }
}

#[tokio::test]
async fn builtin_and_owner_layers_merge_and_the_owner_wins() {
    let settings = RelaySettings::open(startup(
        vec![
            relay("https://builtin.example", 0),
            relay("https://both.example", 0),
        ],
        Some("https://both.example;https://mine.example"),
        None,
    ))
    .await
    .unwrap();
    let snapshot = settings.snapshot();
    let urls: Vec<_> = snapshot
        .entries
        .iter()
        .map(|entry| entry.url.as_str())
        .collect();
    // 内置的 both 被主人配置换了档位，mine 是新加的；builtin 留着。
    assert!(urls.contains(&"https://both.example/"));
    assert!(urls.contains(&"https://mine.example/"));
    assert!(urls.contains(&"https://builtin.example/"));
    assert!(snapshot.locked_by_env);
    assert!(
        snapshot
            .entries
            .iter()
            .all(|entry| entry.source == RelaySource::Env || entry.source == RelaySource::Builtin)
    );
}

#[tokio::test]
async fn the_file_is_not_read_when_the_environment_variable_is_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    std::fs::write(
        &path,
        r#"{"relays":[{"url":"https://file.example","tier":0}]}"#,
    )
    .unwrap();
    let settings = RelaySettings::open(startup(
        vec![relay("https://builtin.example", 0)],
        Some("https://env.example"),
        Some(&path),
    ))
    .await
    .unwrap();
    let urls: Vec<_> = settings
        .snapshot()
        .entries
        .into_iter()
        .map(|entry| entry.url)
        .collect();
    assert!(
        urls.contains(&"https://env.example/".to_owned()),
        "{urls:?}"
    );
    assert!(
        !urls.iter().any(|url| url.contains("file.example")),
        "{urls:?}"
    );
}

#[tokio::test]
async fn saving_writes_the_file_and_a_second_open_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    let settings = RelaySettings::open(startup(
        vec![relay("https://builtin.example", 0)],
        None,
        Some(&path),
    ))
    .await
    .unwrap();
    let update = RelaySettingsUpdate {
        n0: false,
        entries: vec![RelaySettingUpdate {
            url: "https://mine.example".into(),
            token: Some(SecretString::new("secret")),
            quic_port: Some(7843),
            tier: 0,
        }],
    };
    settings.save(update).await.unwrap();
    assert!(!settings.snapshot().n0);
    let reopened = RelaySettings::open(startup(
        vec![relay("https://builtin.example", 0)],
        None,
        Some(&path),
    ))
    .await
    .unwrap();
    let snapshot = reopened.snapshot();
    assert!(!snapshot.n0);
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.url == "https://mine.example/")
        .expect("保存的项读回来了");
    assert_eq!(entry.quic_port, Some(7843));
    assert_eq!(entry.token.as_ref().unwrap().expose(), "secret");
    assert_eq!(entry.source, RelaySource::Settings);
}

#[tokio::test]
async fn an_invalid_file_names_the_path_and_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    std::fs::write(&path, r#"{"relays":[{"url":"http://a.example"}]}"#).unwrap();
    let error = RelaySettings::open(startup(vec![], None, Some(&path)))
        .await
        .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("relays.json"), "{text}");
    assert!(text.contains("https"), "{text}");
}

#[tokio::test]
async fn saving_while_locked_by_the_environment_is_refused() {
    let settings = RelaySettings::open(startup(vec![], Some("https://env.example"), None))
        .await
        .unwrap();
    let error = settings
        .save(RelaySettingsUpdate {
            n0: true,
            entries: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, SaveError::Locked), "{error:?}");
}

#[tokio::test]
async fn the_same_url_twice_in_one_save_is_refused() {
    let settings = RelaySettings::open(startup(vec![], None, None))
        .await
        .unwrap();
    let entry = || RelaySettingUpdate {
        url: "https://a.example".into(),
        token: None,
        quic_port: None,
        tier: 0,
    };
    let error = settings
        .save(RelaySettingsUpdate {
            n0: true,
            entries: vec![entry(), entry()],
        })
        .await
        .unwrap_err();
    assert!(matches!(error, SaveError::Invalid(_)), "{error:?}");
}

#[tokio::test]
async fn with_no_owner_config_the_table_is_exactly_the_n0_group() {
    let settings = RelaySettings::open(startup(vec![], None, None))
        .await
        .unwrap();
    let snapshot = settings
        .save(RelaySettingsUpdate {
            n0: true,
            entries: vec![],
        })
        .await
        .unwrap();
    let expected: Vec<String> = n0_relays(0).map(|relay| relay.url.to_string()).collect();
    let urls: Vec<_> = snapshot
        .entries
        .iter()
        .map(|entry| entry.url.clone())
        .collect();
    assert_eq!(urls, expected);
    assert!(
        snapshot
            .entries
            .iter()
            .all(|entry| entry.source == RelaySource::Builtin)
    );
}

#[tokio::test]
async fn deleting_one_n0_host_sticks_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    let n0: Vec<String> = n0_relays(0).map(|relay| relay.url.to_string()).collect();
    assert!(n0.len() >= 2);
    let settings = RelaySettings::open(startup(vec![], None, Some(&path)))
        .await
        .unwrap();
    let kept: Vec<RelaySettingUpdate> = settings
        .snapshot()
        .entries
        .into_iter()
        .filter(|entry| entry.url != n0[0])
        .map(|entry| RelaySettingUpdate {
            url: entry.url,
            token: entry.token,
            quic_port: entry.quic_port,
            tier: entry.tier,
        })
        .collect();
    settings
        .save(RelaySettingsUpdate {
            n0: true,
            entries: kept,
        })
        .await
        .unwrap();
    let urls: Vec<_> = RelaySettings::open(startup(vec![], None, Some(&path)))
        .await
        .unwrap()
        .snapshot()
        .entries
        .into_iter()
        .map(|entry| entry.url)
        .collect();
    assert!(!urls.contains(&n0[0]), "{urls:?}");
    assert!(urls.contains(&n0[1]), "{urls:?}");
}

#[tokio::test]
async fn turning_n0_off_and_back_on_restores_the_whole_group() {
    // 两种情况都要：n0 只靠兜底加进来（公开构建），和 n0 被构建参数写进了内置表
    // （`WONTON_BUILTIN_RELAYS=…,n0`，部署文档的示例）。
    for n0_in_builtin in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relays.json");
        let builtin: Vec<Relay> = if n0_in_builtin {
            n0_relays(0).collect()
        } else {
            Vec::new()
        };
        let settings = RelaySettings::open(startup(builtin.clone(), None, Some(&path)))
            .await
            .unwrap();
        settings
            .save(RelaySettingsUpdate {
                n0: false,
                entries: vec![],
            })
            .await
            .unwrap();
        settings
            .save(RelaySettingsUpdate {
                n0: true,
                entries: vec![],
            })
            .await
            .unwrap();
        let urls: Vec<_> = RelaySettings::open(startup(builtin, None, Some(&path)))
            .await
            .unwrap()
            .snapshot()
            .entries
            .into_iter()
            .map(|entry| entry.url)
            .collect();
        let expected: Vec<String> = n0_relays(0).map(|relay| relay.url.to_string()).collect();
        assert_eq!(urls, expected, "n0_in_builtin={n0_in_builtin}");
    }
}

#[tokio::test]
async fn a_removed_builtin_url_stays_removed_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    let builtin = vec![
        relay("https://builtin.example", 0),
        relay("https://keep.example", 0),
    ];
    let settings = RelaySettings::open(startup(builtin.clone(), None, Some(&path)))
        .await
        .unwrap();
    settings
        .save(RelaySettingsUpdate {
            n0: true,
            entries: vec![],
        })
        .await
        .unwrap();
    let snapshot = RelaySettings::open(startup(builtin, None, Some(&path)))
        .await
        .unwrap()
        .snapshot();
    let urls: Vec<_> = snapshot
        .entries
        .iter()
        .map(|entry| entry.url.as_str())
        .collect();
    assert!(!urls.contains(&"https://builtin.example/"), "{urls:?}");
}

#[test]
fn no_n0_and_removal_markers_parse() {
    let layer = OwnerLayer::parse("https://a.example,!https://b.example,no-n0").unwrap();
    assert!(!layer.n0);
    assert_eq!(layer.relays.remove().len(), 1);
    let layer = OwnerLayer::parse("n0").unwrap();
    assert!(layer.n0);
    assert!(layer.relays.tier(0).iter().any(|relay| is_n0(&relay.url)));
}

#[test]
fn a_blank_owner_value_is_a_config_error() {
    assert!(OwnerLayer::parse("  ").is_err());
}

fn is_n0(url: &RelayUrl) -> bool {
    wonton_tunnel::relays::is_public_relay(url)
}
