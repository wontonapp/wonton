use std::time::Duration;

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
async fn saving_without_a_file_keeps_the_table_in_memory() {
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
    // 内置 n0 在最后一档兜底；表不是空的。
    eprintln!(
        "{:?}",
        snapshot
            .entries
            .iter()
            .map(|e| (&e.url, e.source))
            .collect::<Vec<_>>()
    );
    assert!(!snapshot.entries.is_empty());
    assert!(
        snapshot
            .entries
            .iter()
            .any(|entry| entry.source == RelaySource::Builtin)
    );
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

#[cfg(unix)]
#[tokio::test]
async fn a_group_readable_file_warns_but_a_0600_file_does_not() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    std::fs::write(&path, r#"{"relays":[]}"#).unwrap();

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let settings = RelaySettings::open(startup(vec![], None, Some(&path)))
        .await
        .unwrap();
    assert!(
        settings
            .snapshot()
            .warnings
            .iter()
            .any(|warning| warning.contains("权限"))
    );
    // 保存会把文件重写成 0600，那条权限警告要跟着消失。
    settings
        .save(RelaySettingsUpdate {
            n0: true,
            entries: Vec::new(),
        })
        .await
        .unwrap();
    assert!(
        !settings
            .snapshot()
            .warnings
            .iter()
            .any(|warning| warning.contains("权限")),
        "保存后文件已是 0600，权限警告还在"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let settings = RelaySettings::open(startup(vec![], None, Some(&path)))
        .await
        .unwrap();
    assert!(
        !settings
            .snapshot()
            .warnings
            .iter()
            .any(|warning| warning.contains("权限"))
    );
}

#[tokio::test]
async fn saving_removes_a_higher_tier_that_the_fallback_had_inserted() {
    use iroh::endpoint::presets;
    use iroh::{RelayMap, RelayMode};

    let settings = RelaySettings::open(startup(
        vec![
            relay("https://first.example.org", 0),
            relay("https://second.example.org", 1),
        ],
        None,
        None,
    ))
    .await
    .unwrap();
    // 这份 map 和 endpoint 共享：插入/撤出都会反映到它上面，用它断言 endpoint 的真实档位。
    let map = RelayMap::empty();
    let endpoint = iroh::Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(map.clone()))
        .bind_addr((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap();
    // 模拟回退任务已经把第二档插进了 map。
    let fallback = wonton_tunnel::relays::spawn_tier_fallback(
        endpoint.clone(),
        settings.table(),
        Duration::from_secs(30),
        None,
    );
    settings.attach(endpoint.clone(), fallback);
    let second: RelayUrl = "https://second.example.org".parse().unwrap();
    endpoint
        .insert_relay(
            second.clone(),
            Arc::new(relay("https://second.example.org", 1).relay_config()),
        )
        .await;
    // 主人存表时删掉了第二档。
    settings
        .save(RelaySettingsUpdate {
            n0: false,
            entries: vec![RelaySettingUpdate {
                url: "https://first.example.org".into(),
                token: None,
                quic_port: None,
                tier: 0,
            }],
        })
        .await
        .unwrap();
    // 删掉的档必须从 endpoint 的 map 里真的撤掉，旧实现只处理第一档会留下它。
    assert!(
        map.contains(&"https://first.example.org/".parse().unwrap()),
        "第一档应保留"
    );
    assert!(
        !map.contains(&second),
        "删掉的第二档还留在 endpoint 的 map 里"
    );
    endpoint.close().await;
}

#[cfg(test)]
async fn empty_test_endpoint() -> iroh::Endpoint {
    use iroh::endpoint::presets;
    use iroh::{RelayMap, RelayMode};

    iroh::Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(RelayMap::empty()))
        .bind_addr((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_closed_endpoint_makes_save_return_apply_and_keeps_the_old_table() {
    let settings = RelaySettings::open(startup(
        vec![relay("https://first.example.org", 0)],
        None,
        None,
    ))
    .await
    .unwrap();
    let endpoint = empty_test_endpoint().await;
    let fallback = wonton_tunnel::relays::spawn_tier_fallback(
        endpoint.clone(),
        settings.table(),
        wonton_tunnel::relays::FALLBACK_AFTER,
        None,
    );
    settings.attach(endpoint.clone(), fallback);
    endpoint.close().await;

    let error = settings
        .save(RelaySettingsUpdate {
            n0: false,
            entries: vec![RelaySettingUpdate {
                url: "https://second.example.org".into(),
                token: None,
                quic_port: None,
                tier: 0,
            }],
        })
        .await
        .unwrap_err();
    assert!(matches!(error, SaveError::Apply(_)), "{error:?}");
    let urls: Vec<_> = settings
        .snapshot()
        .entries
        .iter()
        .map(|entry| entry.url.clone())
        .collect();
    assert!(
        urls.iter().any(|url| url == "https://first.example.org/"),
        "{urls:?}"
    );
    assert!(
        !urls.iter().any(|url| url.contains("second.example.org")),
        "{urls:?}"
    );
    let mapped: Vec<_> = settings
        .state
        .read()
        .unwrap()
        .mapped
        .iter()
        .map(|url| url.to_string())
        .collect();
    assert_eq!(mapped, ["https://first.example.org/"]);
}

#[tokio::test]
async fn a_file_write_failure_returns_store_and_rolls_the_endpoint_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    let settings = RelaySettings::open(startup(vec![], None, Some(&path)))
        .await
        .unwrap();
    let endpoint = empty_test_endpoint().await;
    let fallback = wonton_tunnel::relays::spawn_tier_fallback(
        endpoint.clone(),
        settings.table(),
        wonton_tunnel::relays::FALLBACK_AFTER,
        None,
    );
    settings.attach(endpoint.clone(), fallback);
    let before: Vec<_> = settings
        .state
        .read()
        .unwrap()
        .mapped
        .iter()
        .map(|url| url.to_string())
        .collect();
    // 让写文件那一步失败：把目标路径换成目录，临时文件 persist 不过去。
    std::fs::create_dir(&path).unwrap();

    let error = settings
        .save(RelaySettingsUpdate {
            n0: false,
            entries: vec![RelaySettingUpdate {
                url: "https://second.example.org".into(),
                token: None,
                quic_port: None,
                tier: 0,
            }],
        })
        .await
        .unwrap_err();
    assert!(matches!(error, SaveError::Store(_)), "{error:?}");
    let after: Vec<_> = settings
        .state
        .read()
        .unwrap()
        .mapped
        .iter()
        .map(|url| url.to_string())
        .collect();
    assert_eq!(after, before, "写文件失败后 endpoint 应换回旧表");
    assert!(
        !settings
            .snapshot()
            .entries
            .iter()
            .any(|entry| entry.url.contains("second.example.org")),
        "快照里不应出现没存下的新表"
    );
    endpoint.close().await;
}

#[tokio::test]
async fn deleting_one_n0_entry_stays_deleted_while_the_fallback_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    let settings = RelaySettings::open(startup(vec![], None, Some(&path)))
        .await
        .unwrap();
    let n0: Vec<RelayUrl> = wonton_tunnel::relays::n0_relays(0)
        .map(|relay| relay.url)
        .collect();
    assert!(n0.len() >= 2, "n0 组至少两台：{n0:?}");
    let kept = n0[0].clone();
    settings
        .save(RelaySettingsUpdate {
            n0: true,
            entries: vec![RelaySettingUpdate {
                url: kept.to_string(),
                token: None,
                quic_port: None,
                tier: 0,
            }],
        })
        .await
        .unwrap();
    let reopened = RelaySettings::open(startup(vec![], None, Some(&path)))
        .await
        .unwrap();
    let urls: Vec<String> = reopened
        .snapshot()
        .entries
        .iter()
        .map(|entry| entry.url.clone())
        .collect();
    assert!(urls.iter().any(|url| url == kept.as_str()), "{urls:?}");
    for removed in n0.iter().skip(1) {
        assert!(
            !urls.iter().any(|url| url == removed.as_str()),
            "{} 复活了：{urls:?}",
            removed
        );
    }
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
async fn turning_n0_off_and_back_on_restores_the_whole_group() {
    // 两种情况都要：n0 只靠兜底加进来（公开构建），和 n0 被预置写进了内置表。
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
async fn removed_subpath_urls_are_written_without_a_trailing_slash() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    std::fs::write(&path, r#"{"n0":false,"relays":[]}"#).unwrap();
    let settings = RelaySettings::open(startup(
        vec![relay("https://builtin.example/relay/", 0)],
        None,
        Some(&path),
    ))
    .await
    .unwrap();
    settings
        .save(RelaySettingsUpdate {
            n0: false,
            entries: vec![RelaySettingUpdate {
                url: "https://mine.example/".into(),
                token: None,
                quic_port: None,
                tier: 0,
            }],
        })
        .await
        .unwrap();
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        file["remove"],
        serde_json::json!(["https://builtin.example/relay"])
    );
}

#[tokio::test]
async fn duplicate_subpath_errors_show_the_canonical_url() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relays.json");
    std::fs::write(&path, r#"{"n0":false,"relays":[]}"#).unwrap();
    let settings = RelaySettings::open(startup(vec![], None, Some(&path)))
        .await
        .unwrap();
    let entry = RelaySettingUpdate {
        url: "https://a.example/relay/".into(),
        token: None,
        quic_port: None,
        tier: 0,
    };
    let error = settings
        .save(RelaySettingsUpdate {
            n0: false,
            entries: vec![entry.clone(), entry],
        })
        .await
        .unwrap_err();
    let error = error.to_string();
    assert!(error.contains("https://a.example/relay 出现"), "{error}");
    assert!(!error.contains("/relay/ 出现"), "{error}");
}
