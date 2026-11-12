use super::*;

fn url(text: &str) -> RelayUrl {
    text.parse().unwrap()
}

fn relay(text: &str, tier: u8) -> Relay {
    Relay {
        url: url(text),
        token: None,
        tier,
    }
}

#[test]
fn the_default_table_is_the_n0_group() {
    let table = RelayTable::builtin_n0();
    assert_eq!(RelayTable::parse("n0").unwrap(), table);
    assert_eq!(table.tier_count(), 1);
    assert!(!table.tier(0).is_empty());
    assert!(table.tier(0).iter().all(|relay| relay.tier == 0));
    assert!(table.tier(0).iter().all(|relay| relay.token.is_none()));
}

#[test]
fn a_token_in_the_userinfo_is_taken_out_of_the_url() {
    let table = RelayTable::parse("https://tok@a.example").unwrap();
    assert_eq!(table.tier(0).len(), 1);
    assert_eq!(table.tier(0)[0].url, url("https://a.example/"));
    assert_eq!(table.tier(0)[0].token.as_ref().unwrap().expose(), "tok");
    // 百分号编码的令牌按原文解码。
    let table = RelayTable::parse("https://a%40b%3Ac@example.org;n0").unwrap();
    assert_eq!(table.tier(0)[0].token.as_ref().unwrap().expose(), "a@b:c");
    assert_eq!(table.tier(1).len(), RelayTable::builtin_n0().tier(0).len());
}

#[test]
fn tiers_keep_their_order_and_n0_can_sit_in_any_tier() {
    let table = RelayTable::parse("https://b.example,https://tok@a.example;n0").unwrap();
    assert_eq!(table.tier_count(), 2);
    assert_eq!(table.tier(0)[0].url, url("https://b.example/"));
    assert_eq!(table.tier(0)[1].url, url("https://a.example/"));
    assert_eq!(table.tier(0)[1].token.as_ref().unwrap().expose(), "tok");
    assert_eq!(table.tier(1).len(), RelayTable::builtin_n0().tier(0).len());
}

#[test]
fn a_later_entry_for_the_same_address_replaces_the_earlier_one() {
    let table = RelayTable::parse("https://a.example;n0").unwrap();
    assert_eq!(table.tier_count(), 2);
    // 同一地址再写一次，档位以最后一次为准。
    let table = RelayTable::parse("https://a.example,https://tok@a.example").unwrap();
    assert_eq!(table.tier_count(), 1);
    assert_eq!(table.tier(0).len(), 1);
    assert_eq!(table.tier(0)[0].token.as_ref().unwrap().expose(), "tok");
}

#[test]
fn invalid_values_name_the_offending_item() {
    for (text, expected) in [
        ("", "空的中继项"),
        ("  ", "空的中继项"),
        ("n0;", "空的中继项"),
        ("n0,,n0", "空的中继项"),
        ("notaurl", "不是合法的地址"),
        ("http://a.example", "要用 https"),
        ("https://tok:pw@a.example", "不能有密码"),
        ("https://@a.example", "令牌不能是空"),
        ("https://%ff@a.example", "UTF-8"),
    ] {
        let error = RelayTable::parse(text).unwrap_err();
        assert!(error.contains(expected), "{text}：{error}");
    }
    assert!(
        RelayTable::parse(vec!["n0"; 300].join(";").as_str())
            .unwrap_err()
            .contains("档位太多")
    );
}

#[test]
fn wire_entries_must_not_carry_the_token_in_the_url() {
    let mut entry = relay("https://a.example", 0);
    assert_eq!(entry.validate(), Ok(()));
    entry.url = "https://tok@a.example".parse().unwrap();
    assert!(entry.validate().unwrap_err().contains("userinfo"));
    entry.url = "http://a.example".parse().unwrap();
    assert!(entry.validate().unwrap_err().contains("https"));
    entry = relay("https://a.example", 0);
    entry.token = Some(RelayToken::new(""));
    assert!(entry.validate().unwrap_err().contains("空"));
}

/// 中继地址只认一种写法：FQDN 尾点在哪个入口都规范掉（canonical_url 的说明）。
#[test]
fn relay_urls_are_canonicalized_without_the_fqdn_trailing_dot() {
    assert_eq!(
        canonical_url(url("https://a.example./")),
        url("https://a.example/")
    );
    // 只改域名，保留端口、路径和查询。
    assert_eq!(
        canonical_url(url("https://a.example.:8443/relay?x=1")),
        url("https://a.example:8443/relay?x=1")
    );
    // 没有尾点、IP 地址原样通过。
    assert_eq!(
        canonical_url(url("https://a.example/")),
        url("https://a.example/")
    );
    assert_eq!(
        canonical_url(url("https://127.0.0.1:1/")),
        url("https://127.0.0.1:1/")
    );
}

#[test]
fn every_entry_point_yields_the_same_relay_for_both_spellings() {
    // 配置里的两种写法是同一台中继：去重合并，后面的档位和令牌为准。
    let table = RelayTable::parse("https://a.example;https://tok@a.example.").unwrap();
    assert!(table.tier(0).is_empty());
    assert_eq!(table.tier(1)[0].url, url("https://a.example/"));
    assert_eq!(table.tier(1)[0].token.as_ref().unwrap().expose(), "tok");
    // 从连接信息或配对码组表也一样。
    let table = RelayTable::from_relays(vec![relay("https://a.example./", 0)]);
    assert_eq!(table.tier(0), [relay("https://a.example/", 0)]);
    // iroh 的默认 n0 表里也没有带尾点的写法。
    assert!(
        RelayTable::builtin_n0().relays().iter().all(|relay| !relay
            .url
            .host_str()
            .unwrap_or_default()
            .ends_with('.'))
    );
}

#[test]
fn the_map_only_holds_the_requested_tiers() {
    let table = RelayTable::parse("https://a.example;n0").unwrap();
    let map = relay_map(table.relays(), 1);
    assert_eq!(map.len(), 1);
    assert!(map.contains(&url("https://a.example/")));
    let map = relay_map(table.relays(), 2);
    assert_eq!(map.len(), 1 + RelayTable::builtin_n0().tier(0).len());
    // 令牌进了 iroh 的配置，不在地址里。
    let config = map.get(&url("https://a.example/")).unwrap();
    assert_eq!(config.auth_token.as_deref(), None);
    let table = RelayTable::parse("https://tok@a.example").unwrap();
    let map = relay_map(table.relays(), 1);
    let config = map.get(&url("https://a.example/")).unwrap();
    assert_eq!(config.auth_token.as_deref(), Some("tok"));
}

#[test]
fn fallback_waits_out_the_threshold_before_enabling_the_next_tier() {
    let table = RelayTable::parse("https://a.example;https://b.example;https://c.example").unwrap();
    let mut fallback = TierFallback::new();
    let start = Instant::now();
    let step = |fallback: &mut TierFallback, now, connected: Vec<&str>| {
        fallback.step(
            &table,
            &connected.iter().map(|text| url(text)).collect::<Vec<_>>(),
            &[],
            now,
            Duration::from_secs(30),
        )
    };
    // 顶档连着：什么都不做。
    assert_eq!(step(&mut fallback, start, vec!["https://a.example/"]), None);
    // 全断，但还没到阈值：等。
    assert_eq!(step(&mut fallback, start, vec![]), None);
    assert_eq!(
        step(&mut fallback, start + Duration::from_secs(29), vec![]),
        None
    );
    // 到了阈值：把下一档加进来，并重新计时。
    assert_eq!(
        step(&mut fallback, start + Duration::from_secs(30), vec![]),
        Some(TierAction::Enable(1))
    );
    // 刚加进来的档还没连上，不马上走下一档。
    assert_eq!(
        step(&mut fallback, start + Duration::from_secs(35), vec![]),
        None
    );
    assert_eq!(
        step(&mut fallback, start + Duration::from_secs(65), vec![]),
        Some(TierAction::Enable(2))
    );
    // 全部档都启用后不再加。
    assert_eq!(
        step(&mut fallback, start + Duration::from_secs(100), vec![]),
        None
    );
}

#[test]
fn fallback_removes_a_tier_once_a_lower_one_is_reachable_again() {
    let table = RelayTable::parse("https://a.example;https://b.example;https://c.example").unwrap();
    let mut fallback = TierFallback::new();
    let start = Instant::now();
    let step = |fallback: &mut TierFallback, now, reachable: Vec<&str>| {
        fallback.step(
            &table,
            &[],
            &reachable.iter().map(|text| url(text)).collect::<Vec<_>>(),
            now,
            Duration::from_secs(30),
        )
    };
    // 预热计时，然后逐档启用。
    assert_eq!(step(&mut fallback, start, vec![]), None);
    let t1 = start + Duration::from_secs(30);
    assert_eq!(step(&mut fallback, t1, vec![]), Some(TierAction::Enable(1)));
    assert_eq!(
        step(&mut fallback, t1 + Duration::from_secs(1), vec![]),
        None
    );
    let t2 = t1 + Duration::from_secs(31);
    assert_eq!(step(&mut fallback, t2, vec![]), Some(TierAction::Enable(2)));
    // 低档有端口连得上的中继（顶档恢复了）：最靠后的那档先撤。
    assert_eq!(
        step(&mut fallback, t2, vec!["https://a.example/"]),
        Some(TierAction::Disable(2))
    );
    // 还连得上，继续撤。
    assert_eq!(
        step(&mut fallback, t2, vec!["https://a.example/"]),
        Some(TierAction::Disable(1))
    );
    // 只剩顶档，不再撤。
    assert_eq!(step(&mut fallback, t2, vec!["https://a.example/"]), None);
    // 低档全断时撤档不会发生。
    assert_eq!(step(&mut fallback, t2, vec![]), None);
    // 中间档连得上也算：它一恢复，它后面的档就不需要了。
    let mut fallback = TierFallback::new();
    assert_eq!(step(&mut fallback, t2, vec![]), None);
    let t4 = t2 + Duration::from_secs(30);
    assert_eq!(step(&mut fallback, t4, vec![]), Some(TierAction::Enable(1)));
    assert_eq!(
        step(&mut fallback, t4 + Duration::from_secs(1), vec![]),
        None
    );
    let t5 = t4 + Duration::from_secs(31);
    assert_eq!(step(&mut fallback, t5, vec![]), Some(TierAction::Enable(2)));
    assert_eq!(
        step(&mut fallback, t5, vec!["https://b.example/"]),
        Some(TierAction::Disable(2))
    );
    // 顶档还断着：第二档留着顶班。
    assert_eq!(step(&mut fallback, t5, vec![]), None);
}

#[test]
fn a_recovered_top_tier_resets_the_timer() {
    let table = RelayTable::parse("https://a.example;https://b.example").unwrap();
    let mut fallback = TierFallback::new();
    let start = Instant::now();
    let step = |fallback: &mut TierFallback, now, connected: Vec<&str>| {
        fallback.step(
            &table,
            &connected.iter().map(|text| url(text)).collect::<Vec<_>>(),
            &[],
            now,
            Duration::from_secs(30),
        )
    };
    assert_eq!(step(&mut fallback, start, vec![]), None);
    // 断了 20 秒时恢复一下，计时清零；再断要重新等满 30 秒。
    assert_eq!(
        step(
            &mut fallback,
            start + Duration::from_secs(20),
            vec!["https://a.example/"]
        ),
        None
    );
    assert_eq!(
        step(&mut fallback, start + Duration::from_secs(25), vec![]),
        None
    );
    assert_eq!(
        step(&mut fallback, start + Duration::from_secs(55), vec![]),
        Some(TierAction::Enable(1))
    );
}
