use super::*;

fn url(text: &str) -> RelayUrl {
    text.parse().unwrap()
}

fn relay(text: &str, tier: u8) -> RelayView {
    RelayView {
        url: url(text),
        token: None,
        quic_port: None,
        tier,
    }
}

fn table_of(relays: Vec<RelayView>) -> RelayTable {
    RelayTable::from_views(relays)
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
fn n0_public_relays_are_recognized_by_any_spelling() {
    let n0 = RelayTable::builtin_n0()
        .tier(0)
        .first()
        .unwrap()
        .url
        .clone();
    assert!(is_public_relay(&n0));
    // iroh 内置主机名带 FQDN 尾点，路径地址用同一种写法也要认得。
    let dotted = format!("{n0}.").parse::<RelayUrl>().unwrap();
    assert!(is_public_relay(&dotted));
    assert!(!is_public_relay(&url("https://relay.example.org")));
    assert!(!is_public_relay(&url("https://192.0.2.10/")));
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
    let error = entry.validate().unwrap_err();
    assert!(error.contains("userinfo"), "{error}");
    // 报错串里不能再带一遍令牌。
    assert!(!error.contains("tok"), "{error}");
    entry.url = "http://tok@a.example".parse().unwrap();
    let error = entry.validate().unwrap_err();
    assert!(error.contains("https"), "{error}");
    assert!(!error.contains("tok"), "{error}");
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
    let table = RelayTable::from_views(vec![relay("https://a.example./", 0)]);
    assert_eq!(table.views(), [relay("https://a.example/", 0)]);
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
    let map = relay_map(&table.views(), 1);
    assert_eq!(map.len(), 1);
    assert!(map.contains(&url("https://a.example/")));
    let map = relay_map(&table.views(), 2);
    assert_eq!(map.len(), 1 + RelayTable::builtin_n0().tier(0).len());
    // 令牌进了 iroh 的配置，不在地址里。
    let config = map.get(&url("https://a.example/")).unwrap();
    assert_eq!(config.auth_token.as_deref(), None);
    let table = RelayTable::parse("https://tok@a.example").unwrap();
    let map = relay_map(&table.views(), 1);
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

#[test]
fn a_remove_marker_deletes_an_earlier_address_and_keeps_its_own() {
    let table = RelayTable::parse(
        "https://a.example;https://a.example,https://b.example;!https://a.example",
    )
    .unwrap();
    // 第三档的 `!` 删掉第一档的 a.example，但它自己在第二档写回的 b.example 还在。
    assert_eq!(table.remove(), [url("https://a.example")]);
    assert!(table.tier(0).is_empty());
}

#[test]
fn a_hash_after_the_host_is_the_quic_port() {
    let parsed = relay_from_item("https://tok@a.example#7843", 0).unwrap();
    assert_eq!(parsed.quic_port, Some(7843));
    assert_eq!(parsed.url.as_str(), "https://a.example/");
    assert_eq!(parsed.token.unwrap().expose(), "tok");
    // 不写端口就用 iroh 的默认端口。
    assert_eq!(
        relay_from_item("https://a.example", 0).unwrap().quic_port,
        None
    );
    // 0 不是合法端口。
    assert!(relay_from_item("https://a.example#0", 0).is_err());
}

#[test]
fn an_overlay_removes_and_replaces_by_address() {
    let base = table_of(vec![
        relay("https://a.example", 0),
        relay("https://b.example", 0),
    ]);
    let overlay = table_of(vec![
        relay("https://b.example", 1),
        relay("https://c.example", 0),
    ])
    .with_remove(vec![url("https://a.example")]);
    let merged = overlay.overlay(&base);
    let urls: Vec<_> = merged.relays().iter().map(|r| r.url.as_str()).collect();
    assert_eq!(urls, ["https://c.example/", "https://b.example/"]);
    // a 被删了，b 换到第二档，c 留在第一档。
    assert_eq!(merged.tier(0)[0].url.as_str(), "https://c.example/");
    assert_eq!(merged.tier(1)[0].url.as_str(), "https://b.example/");
}

#[test]
fn n0_can_be_the_last_tier_or_left_out() {
    let custom = table_of(vec![relay("https://a.example", 0)]);
    let with_n0 = custom.with_n0_last(true);
    assert_eq!(with_n0.tier_count(), 2);
    assert!(!with_n0.tier(1).is_empty());
    assert!(is_public_relay(&with_n0.tier(1)[0].url));
    // 已经显式写了 n0 就不重复加。
    let explicit = RelayTable::parse("n0;https://a.example").unwrap();
    let explicit = explicit.with_n0_last(true);
    assert_eq!(explicit.tier_count(), 2);
    // 显式写的 n0 待在原档，后面的地址还在下一档。
    assert!(explicit.tier(0).iter().all(|r| is_public_relay(&r.url)));
    assert!(explicit.tier(1).iter().all(|r| !is_public_relay(&r.url)));
    // 不要 n0 就只是原表。
    assert_eq!(custom.with_n0_last(false).tier_count(), 1);
}

#[test]
fn a_blank_item_is_reported_with_its_tier() {
    let error = RelayTable::parse("https://a.example;;https://b.example").unwrap_err();
    assert!(error.contains("第 2 档"), "{error}");
}

#[test]
fn the_quic_port_reaches_the_relay_config() {
    let mut relay = self::relay("https://a.example", 0);
    relay.quic_port = Some(7843);
    assert_eq!(relay.relay_config().quic.map(|quic| quic.port), Some(7843));
    relay.quic_port = None;
    // 空着就是 iroh 的默认端口（7842），不是关掉 QUIC 地址发现。
    assert_eq!(relay.relay_config().quic.map(|quic| quic.port), Some(7842));
}

#[test]
fn an_at_sign_in_the_query_is_not_mistaken_for_userinfo_in_errors() {
    // 地址合法、只有 query 里带 @：错误/展示不能把它当 userinfo 截断。
    let parsed = relay_from_item("https://a.example/p?email=a@b", 0);
    // 地址本身能解析；redact_item 只应影响提示文案。
    assert!(parsed.is_ok());
    let shown = redact_item("https://a.example/p?email=a@b");
    assert_eq!(shown, "https://a.example/p?email=a@b");
}

#[test]
fn an_invalid_item_does_not_echo_the_token_into_its_error() {
    // 地址里带令牌、又是 http：错误信息里不能再出现令牌原文。
    let error = relay_from_item("http://s3cret@relay.example.org", 0).unwrap_err();
    assert!(error.contains("<redacted>@relay.example.org"), "{error}");
    assert!(!error.contains("s3cret"), "{error}");
}

#[test]
fn url_text_keeps_a_root_slash_and_strips_a_path_slash() {
    assert_eq!(
        relay_url_text(&url("https://a.example")),
        "https://a.example/"
    );
    assert_eq!(
        relay_url_text(&url("https://a.example/relay/")),
        "https://a.example/relay"
    );
    assert_eq!(
        relay_url_text(&url("https://a.example/relay/?email=a@b")),
        "https://a.example/relay?email=a@b"
    );
}

#[test]
fn validate_errors_show_the_canonical_url() {
    let mut relay = relay("https://a.example/relay/", 0);
    relay.url = url("http://a.example/relay/");
    let error = relay.validate().unwrap_err();
    assert!(error.contains("http://a.example/relay："), "{error}");
    assert!(!error.contains("/relay/："), "{error}");

    relay.url = url("https://a.example/relay/");
    relay.quic_port = Some(0);
    let error = relay.validate().unwrap_err();
    assert!(error.contains("https://a.example/relay："), "{error}");
}
