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
        phase: RelaySettingsPhase::Ready,
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
    assert_eq!(view.phase, RelaySettingsPhase::Ready);
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
