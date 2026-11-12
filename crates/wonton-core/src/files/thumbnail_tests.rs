//! 缩略图：读原文件的名额和期限，解码的名额，谁的问题报给谁。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use wonton_protocol::FileFailure;

use super::record::{Accessor, Lookup};
use super::slots::{ALL_PLUGINS, EACH_PLUGIN, PluginSlots};
use super::test_support::Fixture;
use super::tests::{kind, plugin_record, record_of};
use crate::plugin::PluginId;
use crate::plugin::manager::now_secs;

#[tokio::test]
async fn a_slow_plugin_holds_up_no_image_processing() {
    let f = Fixture::new();
    let record = plugin_record(&f, "a.jpg");
    f.host.reads_hang.store(true, Ordering::SeqCst);
    let mut photo = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(8, 8)
        .write_to(&mut photo, image::ImageFormat::Jpeg)
        .unwrap();
    let hung = futures_util::future::join_all((0..4).map(|_| f.files.thumbnail(&record)));
    let other = async {
        tokio::task::yield_now().await;
        f.files.images.thumbnail(photo.get_ref()).await
    };
    tokio::select! {
        _ = hung => panic!("这个插件的读一直不回答"),
        other = tokio::time::timeout(Duration::from_secs(5), other) => {
            assert!(other.expect("别的图片处理不用等慢插件").is_ok());
        }
    }
}

fn with_timeout(f: &mut Fixture, timeout: Duration) {
    Arc::get_mut(&mut f.files).unwrap().thumbnail_read_timeout = timeout;
}

fn with_slot_wait(f: &mut Fixture, wait: Duration) {
    Arc::get_mut(&mut f.files).unwrap().thumbnail_slot_wait = wait;
}

fn local_record(f: &Fixture, path: &str, content: &[u8]) -> super::record::FileRecord {
    let id = f.shared_with_owner(path, content);
    let Lookup::Found(record) = f.files.registry.get(&id, now_secs()) else {
        panic!("刚存的记录");
    };
    record
}

fn jpeg() -> Vec<u8> {
    let mut photo = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(8, 8)
        .write_to(&mut photo, image::ImageFormat::Jpeg)
        .unwrap();
    photo.into_inner()
}

#[tokio::test]
async fn a_thumbnail_read_that_times_out_blames_where_the_bytes_come_from() {
    let mut f = Fixture::new();
    with_timeout(&mut f, Duration::from_millis(50));
    let record = plugin_record(&f, "a.jpg");
    f.host.reads_hang.store(true, Ordering::SeqCst);
    assert_eq!(
        kind(f.files.thumbnail(&record).await),
        FileFailure::PluginUnavailable
    );

    let local = local_record(&f, "photos/a.jpg", &jpeg());
    let stalled = f.files.local_reads_stalled.clone();
    let _stalled = stalled.write().await;
    assert_eq!(
        kind(f.files.thumbnail(&local).await),
        FileFailure::Internal,
        "本地文件读不完不是插件的事"
    );
}

/// 存储卡住时读不回来：超时之后线程和内存都还占着，名额要等读真正结束才还，否则客户端重试会占满阻塞线程池。占满以后后来的报
/// “忙”，不说这张图的存储出了问题。
#[tokio::test]
async fn a_stuck_local_read_keeps_its_slot_until_it_really_ends() {
    let mut f = Fixture::new();
    with_timeout(&mut f, Duration::from_millis(50));
    with_slot_wait(&mut f, Duration::from_millis(50));
    let record = local_record(&f, "photos/a.jpg", &jpeg());
    let stalled = f.files.local_reads_stalled.clone();
    let holding = stalled.write().await;

    for _ in 0..super::MAX_THUMBNAIL_READS {
        assert_eq!(
            kind(f.files.thumbnail(&record).await),
            FileFailure::Internal
        );
    }
    assert_eq!(f.files.local_thumbnail_reads.available_permits(), 0);
    assert_eq!(
        kind(f.files.thumbnail(&record).await),
        FileFailure::ServiceUnavailable
    );

    drop(holding);
    for _ in 0..500 {
        if f.files.local_thumbnail_reads.available_permits() == super::MAX_THUMBNAIL_READS {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        f.files.local_thumbnail_reads.available_permits(),
        super::MAX_THUMBNAIL_READS
    );
    assert!(f.files.thumbnail(&record).await.is_ok());
}

/// 存储卡住、本地读占满名额时，插件的缩略图不受影响，反过来也一样。读的期限从拿到名额算：名额等得再久，好好的读也不算超时。
#[tokio::test]
async fn local_and_plugin_thumbnails_do_not_wait_for_each_other() {
    let mut f = Fixture::new();
    with_timeout(&mut f, Duration::from_millis(200));
    let local = local_record(&f, "photos/a.jpg", &jpeg());
    let plugin = plugin_record(&f, "b.jpg");
    f.host
        .resources
        .lock()
        .unwrap()
        .insert("immich://a".to_owned(), jpeg());

    let stalled = f.files.local_reads_stalled.clone();
    let holding = stalled.write().await;
    for _ in 0..super::MAX_THUMBNAIL_READS {
        assert_eq!(kind(f.files.thumbnail(&local).await), FileFailure::Internal);
    }
    assert!(f.files.thumbnail(&plugin).await.is_ok());
    drop(holding);
    for _ in 0..500 {
        if f.files.local_thumbnail_reads.available_permits() == super::MAX_THUMBNAIL_READS {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    f.host.reads_hang.store(true, Ordering::SeqCst);
    let mut slow = Vec::new();
    for _ in 0..EACH_PLUGIN {
        let (files, plugin) = (f.files.clone(), plugin.clone());
        slow.push(tokio::spawn(async move { files.thumbnail(&plugin).await }));
    }
    while f.files.plugin_thumbnail_reads.free() > ALL_PLUGINS - EACH_PLUGIN {
        tokio::task::yield_now().await;
    }
    assert!(f.files.thumbnail(&local).await.is_ok());
    for read in slow {
        assert_eq!(kind(read.await.unwrap()), FileFailure::PluginUnavailable);
    }
}

/// 所有插件的名额都占满了：这一张可能什么问题都没有。
#[tokio::test]
async fn a_full_pool_reads_as_busy_and_blames_no_one() {
    let mut f = Fixture::new();
    with_slot_wait(&mut f, Duration::from_millis(50));
    let record = plugin_record(&f, "a.jpg");
    let mut taken = Vec::new();
    for n in 0..ALL_PLUGINS {
        let plugin = PluginId::parse(&format!("app.example.p{n}")).unwrap();
        let wait = Duration::from_secs(1);
        taken.push(f.files.plugin_thumbnail_reads.acquire(&plugin, wait).await);
    }
    let failure = f.files.thumbnail(&record).await.unwrap_err();
    assert_eq!(failure.kind, FileFailure::ServiceUnavailable);
}

/// 卡住的 Immich 只占自己的名额：Navidrome 的封面照常，再来的 Immich 请求报 Immich 自己的问题。读的期限不会被等名额用掉。
#[tokio::test]
async fn a_stuck_plugin_only_holds_up_its_own_thumbnails() {
    let mut f = Fixture::new();
    with_slot_wait(&mut f, Duration::from_millis(100));
    let stuck = record_of(&f, "app.immich", "immich://a", "a.jpg");
    let fine = record_of(&f, "app.navidrome", "navidrome://b", "b.jpg");
    f.host
        .resources
        .lock()
        .unwrap()
        .insert("navidrome://b".to_owned(), jpeg());
    Arc::get_mut(&mut f.files).unwrap().plugin_thumbnail_reads = PluginSlots::blaming_any_read();
    *f.host.reads_hang_for.lock().unwrap() = Some("app.immich".to_owned());
    let mut hung = Vec::new();
    for _ in 0..EACH_PLUGIN {
        let (files, stuck) = (f.files.clone(), stuck.clone());
        hung.push(tokio::spawn(async move { files.thumbnail(&stuck).await }));
    }
    while f.files.plugin_thumbnail_reads.free() > ALL_PLUGINS - EACH_PLUGIN {
        tokio::task::yield_now().await;
    }
    let cover = tokio::time::timeout(Duration::from_secs(5), f.files.thumbnail(&fine));
    assert!(cover.await.expect("别的插件不用等").is_ok());
    let failure = f.files.thumbnail(&stuck).await.unwrap_err();
    assert_eq!(failure.kind, FileFailure::PluginUnavailable);
    assert!(failure.to_string().contains("app.immich"), "{failure}");
    for read in hung {
        read.abort();
    }
}

/// 名额被别的读占着：只是忙，不能说这个插件不可用。相册一次要 24 张时就是这样，插件好好的。
#[tokio::test]
async fn a_plugin_that_is_only_busy_is_not_called_unavailable() {
    let mut f = Fixture::new();
    with_slot_wait(&mut f, Duration::from_millis(50));
    let busy = record_of(&f, "app.immich", "immich://a", "a.jpg");
    let plugin = PluginId::parse("app.immich").unwrap();
    let wait = Duration::from_secs(1);
    let _held = [
        f.files.plugin_thumbnail_reads.acquire(&plugin, wait).await,
        f.files.plugin_thumbnail_reads.acquire(&plugin, wait).await,
    ];
    let failure = f.files.thumbnail(&busy).await.unwrap_err();
    assert_eq!(failure.kind, FileFailure::ServiceUnavailable);
}

/// 假的解码器：一直不回答。
fn stuck_decoder(f: &mut Fixture, dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("decode.sh");
    std::fs::write(&script, "#!/bin/sh\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    Arc::get_mut(&mut f.files).unwrap().images = Arc::new(super::ImageWorker::new(script));
}

/// 读完就还插件的名额：解码慢不算到插件头上，解码也有期限，报忙不怪来源。
#[tokio::test]
async fn decoding_holds_no_read_slot_and_reads_as_busy_when_slow() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    stuck_decoder(&mut f, dir.path());
    let files = Arc::get_mut(&mut f.files).unwrap();
    files.thumbnail_decode_timeout = Duration::from_millis(300);
    let record = record_of(&f, "app.immich", "immich://a", "a.jpg");
    f.host
        .resources
        .lock()
        .unwrap()
        .insert("immich://a".to_owned(), jpeg());
    let thumbnail = f.files.thumbnail(&record);
    tokio::pin!(thumbnail);
    let in_decoder = async {
        while f.files.thumbnail_decodes.available_permits() == super::MAX_THUMBNAIL_DECODES {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(
            f.files.plugin_thumbnail_reads.free(),
            ALL_PLUGINS,
            "读完就还"
        );
    };
    tokio::select! {
        finished = &mut thumbnail => panic!("解码器不回答，不该结束：{finished:?}"),
        () = in_decoder => {}
    }
    let failure = thumbnail.await.unwrap_err();
    assert_eq!(failure.kind, FileFailure::ServiceUnavailable);
}

/// 排着等解码的满了就不等了：内存有上限。
#[tokio::test]
async fn too_many_images_waiting_to_decode_read_as_busy() {
    let f = Fixture::new();
    let record = record_of(&f, "app.immich", "immich://a", "a.jpg");
    f.host
        .resources
        .lock()
        .unwrap()
        .insert("immich://a".to_owned(), jpeg());
    let _all = f
        .files
        .thumbnail_decodes
        .clone()
        .acquire_many_owned(super::MAX_THUMBNAIL_DECODES as u32)
        .await
        .unwrap();
    let thumbnail = tokio::time::timeout(Duration::from_secs(5), f.files.thumbnail(&record));
    let failure = thumbnail.await.expect("满了就不等").unwrap_err();
    assert_eq!(failure.kind, FileFailure::ServiceUnavailable);
}

/// 只能经 `resources/read` 读的资源：超过它的上限就没有缩略图，不说有，App 一要又回 502。
#[tokio::test]
async fn a_resource_too_large_to_read_has_no_thumbnail() {
    let f = Fixture::new();
    let record = plugin_record(&f, "a.jpg");
    let info = f.files.info(&record.id, &Accessor::Owner).await.unwrap();
    assert!(info.has_thumbnail, "大小不知道，读了才知道");

    f.declare_size(&record.id, super::MAX_RESOURCE_BYTES as u64 + 1);
    let Lookup::Found(declared) = f.files.registry.get(&record.id, now_secs()) else {
        panic!("刚存的记录");
    };
    let info = f.files.info(&record.id, &Accessor::Owner).await.unwrap();
    assert!(!info.has_thumbnail);
    assert_eq!(
        kind(f.files.thumbnail(&declared).await),
        FileFailure::NotViewable
    );

    // 没声明大小，读了才发现太大。
    f.host.reads_too_large.store(true, Ordering::SeqCst);
    assert_eq!(
        kind(f.files.thumbnail(&record).await),
        FileFailure::NotViewable
    );
}
