//! 文件服务的行为：访问范围、Bot 的编号、原文件被改或被删。

use std::time::Duration;

use wonton_protocol::{FileFailure, FileSource};

use super::record::{Accessor, Lookup, Origin};
use super::resource::RemoteRequest;
use super::share_fs::ShareRef;
use super::test_support::{Fixture, share};
use super::{BotRef, Failure};
use crate::files::test_support::any_langs;
use crate::plugin::PluginId;
use crate::plugin::manager::now_secs;

fn hermes() -> BotRef {
    BotRef {
        name: "hermes".into(),
        plugin: PluginId::parse("app.files").unwrap(),
    }
}

pub(super) fn kind<T: std::fmt::Debug>(result: Result<T, Failure>) -> FileFailure {
    result.unwrap_err().kind
}

#[tokio::test]
async fn a_bot_gets_the_same_id_for_the_same_reference() {
    let f = Fixture::new();
    f.put("photos/a.jpg", b"photo");
    let first = f
        .files
        .register_share(
            &hermes(),
            share("photos/a.jpg"),
            None,
            Some("image/jpeg"),
            &any_langs(),
        )
        .await
        .unwrap();
    assert_eq!(first.name, "a.jpg");
    assert_eq!(
        first.source,
        FileSource::Bot {
            name: "hermes".into(),
            plugin: "app.files".into()
        }
    );
    assert!(first.expires_at.is_some());
    let again = f
        .files
        .register_share(&hermes(), share("photos/a.jpg"), None, None, &any_langs())
        .await
        .unwrap();
    assert_eq!(again.id, first.id);

    let other = BotRef {
        name: "other".into(),
        ..hermes()
    };
    let theirs = f
        .files
        .register_share(&other, share("photos/a.jpg"), None, None, &any_langs())
        .await
        .unwrap();
    assert_ne!(theirs.id, first.id);
}

/// inode 用完是 `ENOSPC`；配额满了是 `EDQUOT`，NAS 上常见。
#[test]
fn a_full_disk_or_quota_on_the_records_reads_as_storage_full() {
    use rustix::io::Errno;
    let failure = |errno: Errno| {
        Failure::from(super::record::RegistryError::Store(
            crate::plugin::store::StoreError::Io {
                path: "records".into(),
                source: std::io::Error::from_raw_os_error(errno.raw_os_error()),
            },
        ))
        .kind
    };
    assert_eq!(failure(Errno::NOSPC), FileFailure::StorageFull);
    assert_eq!(failure(Errno::DQUOT), FileFailure::StorageFull);
    assert_eq!(failure(Errno::IO), FileFailure::Internal);
}

#[tokio::test]
async fn registering_refuses_the_core_s_private_names() {
    let f = Fixture::new();
    f.put("transfers/.wonton-upload-abc", b"half");
    f.put("transfers/.WONTON-trash/3/a.pdf", b"secret");
    for path in [
        "transfers/.wonton-upload-abc",
        "transfers/.WONTON-trash/3/a.pdf",
    ] {
        assert_eq!(
            kind(
                f.files
                    .register_share(&hermes(), share(path), None, None, &any_langs())
                    .await
            ),
            FileFailure::PermissionDenied,
            "{path}"
        );
    }
}

#[tokio::test]
async fn registering_refuses_what_the_bot_should_not_reach() {
    let f = Fixture::new();
    f.put("photos/a.jpg", b"photo");
    std::os::unix::fs::symlink("/etc/passwd", f.shares_dir.join("photos/link")).unwrap();
    assert_eq!(
        kind(
            f.files
                .register_share(&hermes(), share("photos/link"), None, None, &any_langs())
                .await
        ),
        FileFailure::PermissionDenied
    );
    assert_eq!(
        kind(
            f.files
                .register_share(&hermes(), share("photos/missing"), None, None, &any_langs())
                .await
        ),
        FileFailure::FileGone
    );
}

#[tokio::test]
async fn bots_and_devices_outside_the_scope_are_told_apart() {
    let f = Fixture::new();
    f.put("photos/a.jpg", b"photo");
    let record = f
        .files
        .register_share(&hermes(), share("photos/a.jpg"), None, None, &any_langs())
        .await
        .unwrap();
    assert_eq!(
        kind(
            f.files
                .authorize(&record.id, &Accessor::Owner, &any_langs())
        ),
        FileFailure::NotShared
    );
    assert_eq!(
        kind(
            f.files
                .authorize(&record.id, &Accessor::Bot("other".into()), &any_langs())
        ),
        FileFailure::FileUnknown
    );
    assert!(
        f.files
            .authorize(&record.id, &Accessor::Bot("hermes".into()), &any_langs())
            .is_ok()
    );

    let shared = f
        .files
        .share_with_owner(&record.id, "hermes", &any_langs())
        .await
        .unwrap();
    assert_eq!(shared.expires_at, None);
    assert_eq!(
        f.files
            .info(&record.id, &Accessor::Owner, &any_langs())
            .await
            .unwrap()
            .size,
        Some(5)
    );
    assert_eq!(
        kind(
            f.files
                .share_with_owner(&record.id, "other", &any_langs())
                .await
        ),
        FileFailure::FileUnknown,
        "不在范围里的 Bot 分享不了"
    );
}

#[tokio::test]
async fn an_expired_id_is_expired_for_its_bot_and_unknown_for_everyone_else() {
    let f = Fixture::new();
    f.put("photos/a.jpg", b"photo");
    let mut record = f
        .files
        .register_share(&hermes(), share("photos/a.jpg"), None, None, &any_langs())
        .await
        .unwrap();
    record.expires_at = Some(1);
    f.files.registry.save(record.clone()).unwrap();
    assert_eq!(
        kind(
            f.files
                .authorize(&record.id, &Accessor::Bot("hermes".into()), &any_langs())
        ),
        FileFailure::FileExpired
    );
    assert_eq!(
        kind(
            f.files
                .authorize(&record.id, &Accessor::Bot("other".into()), &any_langs())
        ),
        FileFailure::FileUnknown
    );
    assert_eq!(
        kind(
            f.files
                .authorize(&record.id, &Accessor::Owner, &any_langs())
        ),
        FileFailure::FileUnknown
    );
    let fresh = f
        .files
        .register_share(&hermes(), share("photos/a.jpg"), None, None, &any_langs())
        .await
        .unwrap();
    assert_ne!(fresh.id, record.id, "过期后再找一次就有新编号");
}

#[tokio::test]
async fn a_deleted_file_keeps_its_record_so_cards_can_say_why() {
    let f = Fixture::new();
    let id = f.shared_with_owner("photos/a.jpg", b"photo");
    std::fs::remove_file(f.shares_dir.join("photos/a.jpg")).unwrap();
    assert_eq!(
        kind(f.files.info(&id, &Accessor::Owner, &any_langs()).await),
        FileFailure::FileGone
    );
    // 同一路径放了新文件：编号指向路径，下载到的是当前内容。
    f.put("photos/a.jpg", b"new content");
    let info = f
        .files
        .info(&id, &Accessor::Owner, &any_langs())
        .await
        .unwrap();
    assert_eq!(info.size, Some(11));
}

#[tokio::test]
async fn a_shared_file_stays_available_without_its_plugin() {
    let f = Fixture::new();
    f.put("photos/a.jpg", b"photo");
    let record = f
        .files
        .register_share(&hermes(), share("photos/a.jpg"), None, None, &any_langs())
        .await
        .unwrap();
    f.files
        .share_with_owner(&record.id, "hermes", &any_langs())
        .await
        .unwrap();
    // 假核心里没有任何插件在运行。
    assert!(
        f.files
            .info(&record.id, &Accessor::Owner, &any_langs())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn plugin_resources_depend_on_their_plugin() {
    let f = Fixture::new();
    let plugin = PluginId::parse("app.immich").unwrap();
    let id = f.save(
        "a.heic",
        Origin::Plugin {
            plugin: plugin.clone(),
            uri: "immich://a".into(),
            internal_url: None,
        },
        [Accessor::Owner].into(),
    );
    assert_eq!(
        kind(f.files.info(&id, &Accessor::Owner, &any_langs()).await),
        FileFailure::PluginUnavailable
    );
    f.host
        .services
        .lock()
        .unwrap()
        .insert(plugin.to_string(), Default::default());
    let info = f
        .files
        .info(&id, &Accessor::Owner, &any_langs())
        .await
        .unwrap();
    assert_eq!(info.size, None);
    assert_eq!(info.mime_type.as_deref(), Some("image/heic"));
    assert!(!info.has_thumbnail, "解不了 HEIC");
}

#[tokio::test]
async fn internal_urls_must_point_at_the_plugins_own_service() {
    let f = Fixture::new();
    let plugin = PluginId::parse("app.immich").unwrap();
    f.host.services.lock().unwrap().insert(
        plugin.to_string(),
        [("server".to_owned(), "127.0.0.1:1".to_owned())].into(),
    );
    let register = |url: &'static str| {
        let plugin = plugin.clone();
        let files = f.files.clone();
        async move {
            files
                .register_resource(
                    &hermes(),
                    &plugin,
                    "immich://a",
                    Some(url),
                    "a.jpg",
                    None,
                    None,
                    &any_langs(),
                )
                .await
        }
    };
    assert!(register("http://server:2283/a").await.is_ok());
    for url in [
        "http://hydra:4445/admin",
        "http://tuwunel:8008/_synapse/admin",
        "http://127.0.0.1:4445/",
        "https://server/a",
    ] {
        assert_eq!(
            kind(register(url).await),
            FileFailure::PermissionDenied,
            "{url}"
        );
    }
}

#[tokio::test]
async fn names_a_bot_gives_are_cleaned_like_uploaded_ones() {
    let f = Fixture::new();
    f.put("photos/a.jpg", b"photo");
    let named = |name: &'static str| {
        let files = f.files.clone();
        async move {
            files
                .register_share(
                    &hermes(),
                    share("photos/a.jpg"),
                    Some(name),
                    None,
                    &any_langs(),
                )
                .await
                .unwrap()
                .name
        }
    };
    // 第一次登记时的名字留在记录里，所以每次换一个 Bot。
    assert_eq!(named("exe.\u{202E}gpj").await, "exe.gpj");

    let plugin = PluginId::parse("app.immich").unwrap();
    f.host
        .services
        .lock()
        .unwrap()
        .insert(plugin.as_str().to_owned(), Default::default());
    let record = f
        .files
        .register_resource(
            &hermes(),
            &plugin,
            "immich://a",
            None,
            "../a\u{200B}b\u{2066}.png\n",
            None,
            None,
            &any_langs(),
        )
        .await
        .unwrap();
    assert_eq!(record.name, "ab.png");
}

#[tokio::test]
async fn concurrent_registrations_of_one_reference_get_one_id() {
    let f = Fixture::new();
    f.put("photos/a.jpg", b"photo");
    let register = || {
        let files = f.files.clone();
        tokio::spawn(async move {
            files
                .register_share(&hermes(), share("photos/a.jpg"), None, None, &any_langs())
                .await
                .unwrap()
                .id
        })
    };
    let tasks: Vec<_> = (0..8).map(|_| register()).collect();
    let mut ids = Vec::new();
    for task in tasks {
        ids.push(task.await.unwrap());
    }
    ids.dedup();
    assert_eq!(ids.len(), 1, "{ids:?}");
}

#[tokio::test]
async fn deleting_only_touches_uploaded_files() {
    let f = Fixture::new();
    let id = f.shared_with_owner("photos/a.jpg", b"photo");
    f.files
        .delete(&id, "phone".to_owned(), &any_langs())
        .await
        .unwrap();
    assert!(f.shares_dir.join("photos/a.jpg").exists(), "原文件不动");
    assert_eq!(
        kind(f.files.authorize(&id, &Accessor::Owner, &any_langs())),
        FileFailure::FileUnknown
    );

    let id = f.uploaded("b.txt", b"mine");
    f.files
        .delete(&id, "phone".to_owned(), &any_langs())
        .await
        .unwrap();
    assert!(!f.shares_dir.join("transfers/b.txt").exists());

    let id = f.uploaded("c.txt", b"mine");
    let path = f.shares_dir.join("transfers/c.txt");
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"replaced by the owner").unwrap();
    assert_eq!(
        kind(f.files.delete(&id, "phone".to_owned(), &any_langs()).await),
        FileFailure::FileChanged
    );
    assert!(path.exists(), "已经是主人别的文件了");
    assert_eq!(
        kind(f.files.authorize(&id, &Accessor::Owner, &any_langs())),
        FileFailure::FileUnknown,
        "编号照删"
    );

    let id = f.uploaded("d.txt", b"mine");
    std::fs::remove_file(f.shares_dir.join("transfers/d.txt")).unwrap();
    f.files
        .delete(&id, "phone".to_owned(), &any_langs())
        .await
        .unwrap();
}

#[tokio::test]
async fn opening_creates_the_transfers_share_and_refuses_a_symlink() {
    let f = Fixture::new();
    assert!(f.shares_dir.join("transfers").is_dir());
    let dir = tempfile::tempdir().unwrap();
    let shares = dir.path().join("shares");
    std::fs::create_dir(&shares).unwrap();
    std::os::unix::fs::symlink(dir.path(), shares.join("transfers")).unwrap();
    let config = super::FilesConfig {
        dir: dir.path().join("files"),
        shares_dir: shares,
        owner: f.owner,
    };
    let opened = super::Files::open(
        &config,
        f.host.clone(),
        crate::http::HttpClient::new(),
        std::sync::Arc::new(crate::mcp::audit::Audit::new(dir.path().join("audit"))),
        std::sync::Arc::new(crate::plugin::image_worker::ImageWorker::in_process()),
    );
    assert!(matches!(opened, Err(super::OpenError::Dir(_))));
}

pub(super) fn plugin_record(f: &Fixture, name: &str) -> super::record::FileRecord {
    record_of(f, "app.immich", "immich://a", name)
}

pub(super) fn record_of(
    f: &Fixture,
    plugin: &str,
    uri: &str,
    name: &str,
) -> super::record::FileRecord {
    let plugin = PluginId::parse(plugin).unwrap();
    f.host
        .services
        .lock()
        .unwrap()
        .insert(plugin.to_string(), Default::default());
    let origin = Origin::Plugin {
        plugin,
        uri: uri.into(),
        internal_url: None,
    };
    let id = f.save(name, origin, [Accessor::Owner].into());
    let super::record::Lookup::Found(record) = f.files.registry.get(&id, now_secs()) else {
        panic!("刚存的记录");
    };
    record
}

#[tokio::test]
async fn head_fails_like_get_for_a_resource_that_cannot_be_read() {
    let f = Fixture::new();
    let head = || RemoteRequest {
        method: hyper::Method::HEAD,
        ..RemoteRequest::whole()
    };
    let record = plugin_record(&f, "a.bin");
    f.declare_size(&record.id, super::MAX_RESOURCE_BYTES as u64 + 1);
    let Lookup::Found(record) = f.files.registry.get(&record.id, now_secs()) else {
        panic!("刚存的记录");
    };
    for request in [head(), RemoteRequest::whole()] {
        let failed = f
            .files
            .content(&record, request, &any_langs())
            .await
            .err()
            .unwrap();
        assert_eq!(failed.kind, FileFailure::ResourceTooLarge);
    }

    f.declare_size(&record.id, 8);
    let Lookup::Found(record) = f.files.registry.get(&record.id, now_secs()) else {
        panic!("刚存的记录");
    };
    assert!(f.files.content(&record, head(), &any_langs()).await.is_ok());
    let plugin = PluginId::parse("app.immich").unwrap();
    f.host.without_mcp.lock().unwrap().push(plugin.to_string());
    for request in [head(), RemoteRequest::whole()] {
        let failed = f
            .files
            .content(&record, request, &any_langs())
            .await
            .err()
            .unwrap();
        assert_eq!(failed.kind, FileFailure::PluginUnavailable);
    }
}

/// 删除的请求断了：文件照样删完，审计照样写。
#[tokio::test]
async fn a_delete_whose_request_went_away_still_finishes_and_is_audited() {
    let f = Fixture::new();
    let id = f.uploaded("a.txt", b"mine");
    let langs = any_langs();
    let deleting = f.files.delete(&id, "phone".to_owned(), &langs);
    let _ = tokio::time::timeout(Duration::ZERO, deleting).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f
            .audit_lines()
            .iter()
            .any(|line| line["event"] == "file_delete")
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("请求断了也要写审计");
    assert!(!f.shares_dir.join("transfers/a.txt").exists());
    assert_eq!(
        kind(f.files.authorize(&id, &Accessor::Owner, &any_langs())),
        FileFailure::FileUnknown
    );
}

/// 文件删了，目录没能确认落盘：记录照删，审计写明。
#[tokio::test]
async fn a_delete_that_is_not_confirmed_on_disk_says_so_in_the_audit() {
    let f = Fixture::new();
    let id = f.uploaded("a.txt", b"mine");
    super::share_fs::tests::DIR_SYNC_FAILS.set(Some("a.txt".to_owned()));
    let outcome = f.files.delete_now(&id, &any_langs());
    super::share_fs::tests::DIR_SYNC_FAILS.set(None);
    assert_eq!(outcome.unwrap(), super::DELETED_UNSYNCED);
    assert!(!f.shares_dir.join("transfers/a.txt").exists());
    assert_eq!(
        kind(f.files.authorize(&id, &Accessor::Owner, &any_langs())),
        FileFailure::FileUnknown
    );
}

/// 文件还在、删不掉：记录要留着，否则这个文件就再也没人管了。
#[tokio::test]
async fn a_delete_that_fails_keeps_the_record() {
    let f = Fixture::new();
    let elsewhere = f.put("elsewhere/a.txt", b"content");
    // 路径上的 `sub` 换成了软链接：打开时一定被拒绝，以 root 运行也一样。`transfers` 本身没事，别的步骤都走得通。
    let transfers = f.shares_dir.join(super::TRANSFERS);
    std::os::unix::fs::symlink(elsewhere.parent().unwrap(), transfers.join("sub")).unwrap();
    let origin = Origin::Upload {
        reference: ShareRef {
            share: super::TRANSFERS.to_owned(),
            path: vec!["sub".to_owned(), "a.txt".to_owned()],
        },
        placed: super::share_fs::Snapshot {
            inode: 1,
            size: 7,
            modified_ns: 0,
        },
    };
    let id = f.save("a.txt", origin, [Accessor::Owner].into());
    assert_eq!(
        kind(f.files.delete(&id, "phone".to_owned(), &any_langs()).await),
        FileFailure::PermissionDenied
    );
    assert!(matches!(
        f.files.registry.get(&id, now_secs()),
        Lookup::Found(_)
    ));
    assert!(elsewhere.exists());
}

#[tokio::test]
async fn a_restart_finds_the_file_a_crashed_delete_left_hidden() {
    let f = Fixture::new();
    let id = f.uploaded("a.txt", b"content");
    let transfers = f.shares_dir.join(super::TRANSFERS);
    // 崩溃在改名之后。
    let hidden = transfers.join(".wonton-trash/0/a.txt");
    std::fs::rename(transfers.join("a.txt"), &hidden).unwrap();

    let reopened = f.reopen();
    assert!(!hidden.exists());
    assert_eq!(std::fs::read(transfers.join("a.txt")).unwrap(), b"content");
    assert!(
        reopened
            .info(&id, &Accessor::Owner, &any_langs())
            .await
            .is_ok()
    );
}
