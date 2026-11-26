//! 中继表的本地编辑规则：地址校验、加/改/删/挪档、令牌为空串的清掉语义、只读表。

use super::*;

fn entry(url: &str, tier: u8) -> RelaySettingEntry {
    RelaySettingEntry {
        url: url.into(),
        token: None,
        quic_port: None,
        tier,
        source: RelaySource::Signed,
        state: RelayState::Unknown,
        editable: true,
    }
}

fn view(entries: Vec<RelaySettingEntry>) -> RelaySettingsView {
    RelaySettingsView {
        phase: Some(RelaySettingsPhase::Ready),
        entries,
        ..Default::default()
    }
}

fn urls(view: &RelaySettingsView) -> Vec<&str> {
    view.entries
        .iter()
        .map(|entry| entry.url.as_str())
        .collect()
}

#[test]
fn add_appends_at_the_last_tier_by_default() {
    let mut view = view(vec![entry("https://first.example.org/", 0)]);
    add(
        &mut view,
        "https://second.example.org".into(),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        urls(&view),
        ["https://first.example.org/", "https://second.example.org/"]
    );
    assert!(view.entries[1].editable);
    assert_eq!(view.entries[1].source, RelaySource::Settings);
}

#[test]
fn add_normalizes_the_address_and_trims_a_blank_token() {
    let mut view = view(Vec::new());
    add(
        &mut view,
        "  https://relay.example.org  ".into(),
        Some(SecretString::new("  ")),
        Some(7842),
        Some(3),
    )
    .unwrap();
    assert_eq!(view.entries[0].url, "https://relay.example.org/");
    assert!(view.entries[0].token.is_none());
    assert_eq!(view.entries[0].quic_port, Some(7842));
    assert_eq!(view.entries[0].tier, 3);
}

#[test]
fn add_rejects_bad_addresses_and_duplicates() {
    let mut view = view(vec![entry("https://relay.example.org/", 0)]);
    for url in [
        "http://relay.example.org",
        "not a url",
        "https://token@relay.example.org",
    ] {
        assert!(
            matches!(
                add(&mut view, url.into(), None, None, None),
                Err(CoreError::InvalidInput { .. })
            ),
            "{url}"
        );
    }
    assert!(matches!(
        add(
            &mut view,
            "https://relay.example.org".into(),
            None,
            None,
            None
        ),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[test]
fn addresses_drop_the_dns_root_dot() {
    let mut view = view(Vec::new());
    add(
        &mut view,
        "https://Relay.Example.org.".into(),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(urls(&view), ["https://relay.example.org/"]);
    // 带根点的写法和不带的算同一条。
    assert!(matches!(
        add(
            &mut view,
            "https://relay.example.org".into(),
            None,
            None,
            None
        ),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[test]
fn update_keeps_the_token_when_none_and_clears_it_on_an_empty_string() {
    let mut view = view(vec![entry("https://relay.example.org/", 0)]);
    view.entries[0].token = Some(SecretString::new("secret"));

    update(
        &mut view,
        "https://relay.example.org".into(),
        None,
        Some(9000),
        Some(1),
    )
    .unwrap();
    assert_eq!(
        view.entries[0].token.as_ref().map(SecretString::expose),
        Some("secret")
    );
    assert_eq!(view.entries[0].quic_port, Some(9000));
    assert_eq!(view.entries[0].tier, 1);

    update(
        &mut view,
        "https://relay.example.org".into(),
        Some(SecretString::new("")),
        None,
        None,
    )
    .unwrap();
    assert!(view.entries[0].token.is_none());
    // 端口空着表示清掉，档位空着表示不动。
    assert_eq!(view.entries[0].quic_port, None);
    assert_eq!(view.entries[0].tier, 1);
}

#[test]
fn update_resorts_by_tier() {
    let mut view = view(vec![
        entry("https://first.example.org/", 0),
        entry("https://second.example.org/", 1),
    ]);
    update(
        &mut view,
        "https://first.example.org".into(),
        None,
        None,
        Some(2),
    )
    .unwrap();
    assert_eq!(
        urls(&view),
        ["https://second.example.org/", "https://first.example.org/"]
    );
}

#[test]
fn remove_reports_a_missing_entry() {
    let mut view = view(vec![entry("https://relay.example.org/", 0)]);
    remove(&mut view, "https://relay.example.org".into()).unwrap();
    assert!(view.entries.is_empty());
    assert!(matches!(
        remove(&mut view, "https://relay.example.org".into()),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[test]
fn move_puts_the_entry_before_the_given_one() {
    let mut view = view(vec![
        entry("https://first.example.org/", 0),
        entry("https://second.example.org/", 1),
        entry("https://third.example.org/", 1),
    ]);
    move_entry(
        &mut view,
        "https://third.example.org".into(),
        1,
        Some("https://second.example.org/".into()),
    )
    .unwrap();
    assert_eq!(
        urls(&view),
        [
            "https://first.example.org/",
            "https://third.example.org/",
            "https://second.example.org/"
        ]
    );
}

#[test]
fn move_without_a_before_goes_to_the_end_of_the_tier() {
    let mut view = view(vec![
        entry("https://first.example.org/", 0),
        entry("https://second.example.org/", 1),
        entry("https://third.example.org/", 2),
    ]);
    move_entry(&mut view, "https://first.example.org".into(), 1, None).unwrap();
    assert_eq!(
        urls(&view),
        [
            "https://second.example.org/",
            "https://first.example.org/",
            "https://third.example.org/"
        ]
    );
}

#[test]
fn move_rejects_a_missing_before() {
    let mut view = view(vec![entry("https://first.example.org/", 0)]);
    assert!(matches!(
        move_entry(
            &mut view,
            "https://first.example.org".into(),
            1,
            Some("https://gone.example.org/".into())
        ),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[test]
fn move_before_itself_puts_it_at_the_front_of_the_tier() {
    let mut view = view(vec![
        entry("https://first.example.org/", 0),
        entry("https://second.example.org/", 1),
        entry("https://third.example.org/", 2),
    ]);
    move_entry(
        &mut view,
        "https://third.example.org".into(),
        1,
        Some("https://third.example.org/".into()),
    )
    .unwrap();
    assert_eq!(
        urls(&view),
        [
            "https://first.example.org/",
            "https://third.example.org/",
            "https://second.example.org/"
        ]
    );
}

#[test]
fn move_across_tiers_stays_before_a_before_in_a_higher_tier() {
    let mut view = view(vec![
        entry("https://first.example.org/", 0),
        entry("https://second.example.org/", 1),
        entry("https://third.example.org/", 2),
    ]);
    // third 挪进 0 档、放在 second 前面：second 在更高档，插在目标档的最后，排完仍排在它前面。
    move_entry(
        &mut view,
        "https://third.example.org".into(),
        0,
        Some("https://second.example.org/".into()),
    )
    .unwrap();
    assert_eq!(
        urls(&view),
        [
            "https://first.example.org/",
            "https://third.example.org/",
            "https://second.example.org/"
        ]
    );
}

#[test]
fn move_across_tiers_goes_to_the_front_of_the_tier_when_before_is_lower() {
    let mut view = view(vec![
        entry("https://first.example.org/", 0),
        entry("https://second.example.org/", 1),
        entry("https://third.example.org/", 1),
        entry("https://fourth.example.org/", 2),
    ]);
    // fourth 挪进 1 档、放在 first 前面：first 在更低档，按档位落在 1 档的开头。
    move_entry(
        &mut view,
        "https://fourth.example.org/".into(),
        1,
        Some("https://first.example.org/".into()),
    )
    .unwrap();
    assert_eq!(
        urls(&view),
        [
            "https://first.example.org/",
            "https://fourth.example.org/",
            "https://second.example.org/",
            "https://third.example.org/"
        ]
    );
}

#[test]
fn a_locked_table_rejects_every_edit() {
    let mut view = view(vec![entry("https://relay.example.org/", 0)]);
    view.locked_by_env = true;
    assert!(matches!(
        add(
            &mut view,
            "https://second.example.org".into(),
            None,
            None,
            None
        ),
        Err(CoreError::NotReady { .. })
    ));
    assert!(matches!(
        update(
            &mut view,
            "https://relay.example.org".into(),
            None,
            None,
            None
        ),
        Err(CoreError::NotReady { .. })
    ));
    assert!(matches!(
        remove(&mut view, "https://relay.example.org".into()),
        Err(CoreError::NotReady { .. })
    ));
    assert!(matches!(
        move_entry(&mut view, "https://relay.example.org".into(), 1, None),
        Err(CoreError::NotReady { .. })
    ));
    assert!(matches!(
        set_n0(&mut view, true),
        Err(CoreError::NotReady { .. })
    ));
}

#[test]
fn an_entry_the_core_marks_read_only_cannot_be_changed() {
    let mut view = view(vec![entry("https://relay.example.org/", 0)]);
    view.entries[0].editable = false;
    assert!(matches!(
        update(
            &mut view,
            "https://relay.example.org".into(),
            None,
            Some(1),
            None
        ),
        Err(CoreError::NotReady { .. })
    ));
    assert!(matches!(
        remove(&mut view, "https://relay.example.org".into()),
        Err(CoreError::NotReady { .. })
    ));
    assert!(matches!(
        move_entry(&mut view, "https://relay.example.org".into(), 1, None),
        Err(CoreError::NotReady { .. })
    ));
}

#[test]
fn applying_a_snapshot_clears_dirty_and_problems() {
    let mut view = RelaySettingsView {
        dirty: true,
        load_problem: Some("load".into()),
        save_problem: Some("save".into()),
        ..Default::default()
    };
    let snapshot = RelaySettingsSnapshot {
        entries: vec![entry("https://relay.example.org/", 0)],
        n0: true,
        locked_by_env: true,
        warnings: vec!["warning".into()],
    };
    apply_snapshot(&mut view, &snapshot);
    assert_eq!(view.phase, Some(RelaySettingsPhase::Ready));
    assert_eq!(urls(&view), ["https://relay.example.org/"]);
    assert!(view.n0);
    assert!(view.locked_by_env);
    assert_eq!(view.warnings, ["warning"]);
    assert!(!view.dirty);
    assert!(view.load_problem.is_none());
    assert!(view.save_problem.is_none());
}

#[test]
fn update_from_carries_the_whole_table() {
    let mut view = view(vec![entry("https://relay.example.org/", 0)]);
    view.entries[0].token = Some(SecretString::new("secret"));
    view.n0 = true;
    let update = update_from(&view);
    assert!(update.n0);
    assert_eq!(update.entries.len(), 1);
    assert_eq!(update.entries[0].url, "https://relay.example.org/");
    assert_eq!(
        update.entries[0].token.as_ref().map(SecretString::expose),
        Some("secret")
    );
    assert_eq!(update.entries[0].tier, 0);
}

async fn idle_home() -> (tempfile::TempDir, std::sync::Arc<super::super::HomeLink>) {
    let dir = tempfile::tempdir().unwrap();
    let storage = super::super::storage::Storage::open(dir.path(), dir.path()).unwrap();
    let (_method_tx, method_rx) =
        tokio::sync::watch::channel(wonton_protocol::PushMethod::default());
    let home = super::super::HomeLink::new(
        tokio::runtime::Handle::current(),
        wonton_tunnel::sharing::ProxyHub::bind().await.unwrap(),
        storage,
        super::super::Network::DirectOnly,
        None,
        method_rx,
    )
    .unwrap();
    (dir, home)
}

#[tokio::test]
async fn a_failed_edit_while_loading_keeps_the_in_flight_load() {
    let (_dir, home) = idle_home().await;
    home.view
        .send_modify(|view| view.relays.phase = Some(RelaySettingsPhase::Loading));
    let before = home.lock_relays().epoch;
    let error = home
        .edit_relays(|_| {
            Err(CoreError::NotReady {
                reason: "boom".into(),
            })
        })
        .unwrap_err();
    assert!(matches!(error, CoreError::NotReady { .. }));
    // 在途的 GET 回来时 epoch 必须还对得上，否则这份设置永远停在 Loading。
    assert_eq!(home.lock_relays().epoch, before);
    assert_eq!(home.view().relays.phase, Some(RelaySettingsPhase::Loading));
}

#[tokio::test]
async fn a_successful_edit_while_loading_discards_the_in_flight_load() {
    let (_dir, home) = idle_home().await;
    home.view
        .send_modify(|view| view.relays.phase = Some(RelaySettingsPhase::Loading));
    let before = home.lock_relays().epoch;
    home.edit_relays(|_| Ok(())).unwrap();
    assert_eq!(home.view().relays.phase, Some(RelaySettingsPhase::Ready));
    // 成功的编辑作废在途的读，免得旧快照回来盖掉刚改的这份。
    assert_ne!(home.lock_relays().epoch, before);
}

#[tokio::test]
async fn an_unknown_phase_can_still_be_edited_and_becomes_ready() {
    let (_dir, home) = idle_home().await;
    home.view
        .send_modify(|view| view.relays.phase = Some(RelaySettingsPhase::Other));
    home.edit_relays(|view| {
        view.entries = Vec::new();
        Ok(())
    })
    .unwrap();
    assert_eq!(home.view().relays.phase, Some(RelaySettingsPhase::Ready));
    assert!(home.view().relays.dirty);
}

#[test]
fn add_takes_the_quic_port_from_the_fragment() {
    let mut view = view(Vec::new());
    add(
        &mut view,
        "https://relay.example.org.#7843".into(),
        None,
        None,
        Some(0),
    )
    .unwrap();
    assert_eq!(view.entries[0].url, "https://relay.example.org/");
    assert_eq!(view.entries[0].quic_port, Some(7843));
}

#[test]
fn an_explicit_port_wins_over_the_fragment() {
    let mut view = view(Vec::new());
    add(
        &mut view,
        "https://relay.example.org#7843".into(),
        None,
        Some(9000),
        Some(0),
    )
    .unwrap();
    assert_eq!(view.entries[0].quic_port, Some(9000));
}

#[tokio::test]
async fn saving_before_any_table_loaded_is_refused() {
    let (_dir, home) = idle_home().await;
    home.view.send_modify(|view| {
        view.relays.phase = Some(RelaySettingsPhase::Ready);
        view.proxy = Some(ProxySettings {
            host: "127.0.0.1".into(),
            port: 1,
            username: "u".into(),
            password: "p".into(),
            domain: "wonton.localhost".into(),
        });
    });
    *home.core_url.lock().unwrap() = Some("http://127.0.0.1:1/".parse().unwrap());
    // 一次都没取到过表，entries 是空的；交上去等于把服务端的项全删了。
    let error = home.save_relays().unwrap_err();
    assert!(matches!(error, CoreError::NotReady { .. }), "{error:?}");
    assert_eq!(home.view().relays.phase, Some(RelaySettingsPhase::Ready));
}

#[test]
fn a_subpath_snapshot_without_a_trailing_slash_can_be_updated_and_removed() {
    let mut view = view(vec![
        entry("https://relay.example.org/relay", 0),
        entry("https://other.example.org/", 1),
    ]);
    update(
        &mut view,
        "https://relay.example.org/relay".into(),
        None,
        None,
        Some(2),
    )
    .unwrap();
    assert_eq!(view.entries[1].tier, 2);
    remove(&mut view, "https://relay.example.org/relay".into()).unwrap();
    assert_eq!(urls(&view), ["https://other.example.org/"]);
}
