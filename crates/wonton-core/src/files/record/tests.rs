use super::*;

fn id(byte: u8) -> FileId {
    FileId::from_bytes([byte; 16])
}

fn share_origin(path: &str) -> Origin {
    Origin::Share {
        reference: ShareRef::parse(&format!("wonton-share:photos/{path}")).unwrap(),
    }
}

fn bot_record(id: FileId, bot: &str, origin: Origin, now: u64) -> FileRecord {
    FileRecord {
        id,
        name: "a.jpg".into(),
        mime_type: Some("image/jpeg".into()),
        declared_size: None,
        source: FileSource::Bot {
            name: bot.into(),
            plugin: "app.files".into(),
        },
        origin,
        access: [Accessor::Bot(bot.into())].into(),
        owner_granted: Default::default(),
        registered: None,
        created_at: now,
        expires_at: Some(now + BOT_TTL_SECS),
    }
}

#[test]
fn accessors_are_written_as_owner_and_bot_name() {
    let accessors = [Accessor::Owner, Accessor::Bot("hermes".into())];
    let json = serde_json::to_value(&accessors).unwrap();
    assert_eq!(json, serde_json::json!(["owner", "bot:hermes"]));
    let back: Vec<Accessor> = serde_json::from_value(json).unwrap();
    assert_eq!(back, accessors);
    assert!("bot:".parse::<Accessor>().is_err());
    assert!("family:alice".parse::<Accessor>().is_err());
}

#[test]
fn records_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("records");
    let record = bot_record(id(1), "hermes", share_origin("a.jpg"), 100);
    {
        let registry = Registry::open(path.clone()).unwrap();
        registry.save(record.clone()).unwrap();
        registry.grant(&id(1), Accessor::Owner, 100).unwrap();
    }
    let registry = Registry::open(path).unwrap();
    let Lookup::Found(found) = registry.get(&id(1), 100) else {
        panic!("重启后记录应该还在");
    };
    assert_eq!(found.origin, record.origin);
    assert!(found.access.contains(&Accessor::Owner));
    assert_eq!(found.expires_at, None);
}

#[test]
fn a_damaged_record_stops_startup_and_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("records");
    drop(Registry::open(path.clone()).unwrap());
    std::fs::write(path.join("broken.json"), "{").unwrap();
    let Err(error) = Registry::open(path) else {
        panic!("应该报错");
    };
    assert!(error.to_string().contains("broken.json"), "{error}");
}

#[test]
fn unshared_bot_records_expire_then_disappear() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::open(dir.path().join("records")).unwrap();
    registry
        .save(bot_record(id(1), "hermes", share_origin("a.jpg"), 100))
        .unwrap();
    let expiry = 100 + BOT_TTL_SECS;
    assert!(matches!(registry.get(&id(1), expiry - 1), Lookup::Found(_)));
    assert!(matches!(registry.get(&id(1), expiry), Lookup::Expired(_)));

    assert_eq!(registry.purge_expired(expiry).unwrap(), 0);
    assert_eq!(
        registry.purge_expired(expiry + EXPIRED_KEEP_SECS).unwrap(),
        1
    );
    assert_eq!(registry.get(&id(1), expiry), Lookup::Unknown);
    assert!(
        std::fs::read_dir(dir.path().join("records"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
fn sharing_with_the_owner_stops_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::open(dir.path().join("records")).unwrap();
    registry
        .save(bot_record(id(1), "hermes", share_origin("a.jpg"), 100))
        .unwrap();
    let granted = registry
        .grant(&id(1), Accessor::Owner, 200)
        .unwrap()
        .unwrap();
    assert_eq!(granted.expires_at, None);
    assert!(matches!(registry.get(&id(1), u64::MAX), Lookup::Found(_)));
    assert_eq!(registry.purge_expired(u64::MAX).unwrap(), 0);
}

#[test]
fn granting_a_bot_keeps_the_clock_and_an_expired_id_cannot_be_granted() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::open(dir.path().join("records")).unwrap();
    registry
        .save(bot_record(id(1), "hermes", share_origin("a.jpg"), 100))
        .unwrap();
    let other = Accessor::Bot("other".into());
    let granted = registry.grant(&id(1), other.clone(), 100).unwrap().unwrap();
    assert!(granted.access.contains(&other));
    assert_eq!(granted.expires_at, Some(100 + BOT_TTL_SECS));
    let late = 100 + BOT_TTL_SECS;
    assert_eq!(registry.grant(&id(1), Accessor::Owner, late).unwrap(), None);
    assert_eq!(registry.grant(&id(9), Accessor::Owner, 100).unwrap(), None);
}

fn register(registry: &Registry, new: u8, bot: &str, origin: &Origin, now: u64) -> FileId {
    registry
        .register_bot(
            bot,
            origin,
            "app.files",
            now,
            |_| {},
            || bot_record(id(new), bot, origin.clone(), now),
        )
        .unwrap()
        .id
}

#[test]
fn only_the_same_bot_and_reference_find_the_old_record() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::open(dir.path().join("records")).unwrap();
    let origin = share_origin("a.jpg");
    assert_eq!(register(&registry, 1, "hermes", &origin, 100), id(1));
    assert_eq!(register(&registry, 2, "hermes", &origin, 100), id(1));
    assert_eq!(register(&registry, 3, "other", &origin, 100), id(3));
    let other_file = share_origin("b.jpg");
    assert_eq!(register(&registry, 4, "hermes", &other_file, 100), id(4));
    assert_eq!(
        register(&registry, 5, "hermes", &origin, 100 + BOT_TTL_SECS),
        id(5),
        "过期了要给新编号"
    );
}

/// 找、改、写是一步：并发的两次操作互不覆盖，同一个引用只有一个编号。
#[test]
fn concurrent_registering_and_granting_keep_every_change() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::open(dir.path().join("records")).unwrap();
    let origin = share_origin("a.jpg");
    let first = register(&registry, 0, "hermes", &origin, 100);
    std::thread::scope(|scope| {
        for n in 1..=8u8 {
            let (registry, origin, first) = (&registry, &origin, &first);
            scope.spawn(move || {
                let got = register(registry, n, "hermes", origin, 100);
                assert_eq!(&got, first);
                let friend = Accessor::Bot(format!("friend{n}"));
                registry.grant(first, friend, 100).unwrap().unwrap();
            });
        }
    });
    let Lookup::Found(record) = registry.get(&first, 100) else {
        panic!("记录应该还在");
    };
    assert_eq!(record.access.len(), 9, "{:?}", record.access);
    let reopened = Registry::open(dir.path().join("records")).unwrap();
    assert_eq!(reopened.get(&first, 100), Lookup::Found(record));
}

/// 查得快靠索引，所以新建、过期、删除、重启之后都要查得对。
#[test]
fn a_bot_finds_its_record_through_every_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("records");
    let registry = Registry::open(path.clone()).unwrap();
    let origin = share_origin("a.jpg");
    let register = |registry: &Registry, new: u8, bot: &str, now: u64| {
        let fresh = || bot_record(id(new), bot, origin.clone(), now);
        registry
            .register_bot(bot, &origin, "app.files", now, |_| {}, fresh)
            .unwrap()
            .id
    };

    assert_eq!(register(&registry, 1, "hermes", 100), id(1));
    assert_eq!(register(&registry, 2, "hermes", 100), id(1));
    // 别的 Bot、别的来源各是各的。
    assert_eq!(register(&registry, 3, "other", 100), id(3));
    let elsewhere = share_origin("b.jpg");
    let fresh = || bot_record(id(4), "hermes", elsewhere.clone(), 100);
    let found = registry.register_bot("hermes", &elsewhere, "app.files", 100, |_| {}, fresh);
    assert_eq!(found.unwrap().id, id(4));

    // 过期的不算；旧记录清掉之后，新记录还查得到。
    let later = 100 + BOT_TTL_SECS;
    assert_eq!(register(&registry, 5, "hermes", later), id(5));
    registry.purge_expired(later + EXPIRED_KEEP_SECS).unwrap();
    let much_later = later + 1;
    assert_eq!(register(&registry, 6, "hermes", much_later), id(5));

    drop(registry);
    let registry = Registry::open(path).unwrap();
    assert_eq!(register(&registry, 7, "hermes", much_later), id(5));
    registry.remove(&id(5)).unwrap();
    assert_eq!(register(&registry, 8, "hermes", much_later), id(8));
}

/// `fsync` 的错误只报告一次：第一次失败要交到调用方手里，重试时新的一次写才说明落盘了。
#[test]
fn a_failed_directory_sync_reaches_the_caller_and_a_rewrite_settles_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("files");
    let registry = Registry::open(path.clone()).unwrap();
    let record = bot_record(id(1), "hermes", share_origin("a.jpg"), 100);

    crate::plugin::store::DIR_SYNC_FAILURES
        .lock()
        .unwrap()
        .insert(path.clone(), 2);
    assert!(!registry.save_synced(record.clone()).unwrap());
    assert!(!registry.resave(&record.id).unwrap());
    assert!(registry.resave(&record.id).unwrap());
    assert!(matches!(registry.get(&record.id, 100), Lookup::Found(_)));
    assert!(registry.resave(&id(2)).unwrap(), "没有的记录没什么可丢的");
}
