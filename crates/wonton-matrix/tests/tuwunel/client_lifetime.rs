//! 同步过的 Client 必须能整体释放：进程内重建会话时旧实例不再滞留（#67，归档 #816）。
//!
//! 登录后至少 `sync_once` 一次，SDK 会建 `LatestEvents` 并让它的监听任务持有 `SendQueue`
//! （补丁见 `vendor/README.md` 的 LatestEvents 一节）。任务把客户端按住，就永远等不到自己该退出的通道关闭。
//! 丢掉会话和手里的全部强引用，弱引用必须取回 `None`。

use std::time::Duration;

use matrix_sdk::{
    WeakClient,
    config::SyncSettings,
    cross_process_lock::{CrossProcessLockConfig, CrossProcessLockGuard},
};
use wonton_matrix::testkit::{TempStore, TestServer, eventually, unique};
use wonton_matrix::{MatrixSession, Recovery, SavedSession, StoreConfig, Verification};

/// 释放发生在 drop 之后的任务收尾，给一个有上限的等待窗口。
const RELEASE: Duration = Duration::from_secs(5);

/// 丢掉会话、停同步，等 SDK 内部把客户端也放掉。
async fn released(session: MatrixSession) -> bool {
    let weak = session.client().downgrade();
    session.stop_sync().await;
    drop(session);
    gone(&weak, RELEASE).await
}

async fn gone(weak: &WeakClient, window: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + window;
    while weak.get().is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    weak.get().is_none()
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_synced_client_is_released() {
    let server = TestServer::start().await;
    let session = server
        .login_via(&server.url, &unique("leak"), "g", StoreConfig::InMemory)
        .await;
    session
        .client()
        .sync_once(SyncSettings::default().timeout(Duration::from_secs(1)))
        .await
        .expect("同步一次");

    assert!(
        released(session).await,
        "同步过的 Client 在 {RELEASE:?} 后仍被 SDK 内部持有"
    );
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_client_that_recovered_its_keys_is_released() {
    let server = TestServer::start().await;
    let user = unique("leak");
    let first = server
        .login_via(&server.url, &user, "第一台", StoreConfig::InMemory)
        .await;
    first.start_sync().await.expect("第一台同步");
    eventually("第一台已验证", first.encryption_states(), |state| {
        (state.verification == Verification::Verified).then_some(())
    })
    .await;
    // 恢复密钥要等备份上传完才返回。
    let key = first.enable_recovery().await.expect("开启恢复");
    first.stop_sync().await;
    drop(first);

    let session = server
        .login_via(&server.url, &user, "第二台", StoreConfig::InMemory)
        .await;
    session.start_sync().await.expect("第二台同步");
    eventually("待恢复", session.encryption_states(), |state| {
        (state.recovery == Recovery::Incomplete).then_some(())
    })
    .await;
    session.recover(&key).await.expect("输入恢复密钥");

    assert!(
        released(session).await,
        "恢复过密钥的 Client 在 {RELEASE:?} 后仍被 SDK 内部持有"
    );
}

/// 真实的进程内重建：旧会话在新会话打开同一个存储之前必须已经放掉，新会话才能接着用。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_client_is_released_before_a_rebuild_on_the_same_store() {
    let server = TestServer::start().await;
    let user = unique("leak");
    let store = TempStore::create();
    let old = server
        .login_via(&server.url, &user, "old", store.config())
        .await;
    old.start_sync().await.expect("旧会话同步");
    let weak = old.client().downgrade();
    let saved = old.saved().expect("登录状态");
    old.stop_sync().await;
    drop(old);
    assert!(
        gone(&weak, RELEASE).await,
        "旧会话丢掉引用 {RELEASE:?} 后还活着"
    );

    let new = MatrixSession::restore(&server.config("old", store.config()), saved)
        .await
        .expect("同一个存储上重建会话");
    new.start_sync().await.expect("新会话同步");
}

/// 换会话时旧同步没在期限内停完、挪到后台接着停：旧 Client 还活着期间，同一个存储上新建的 Client 不能沿用它的
/// 持有者名，否则加密状态不重读、消息解不开（#804）。
// 多线程：拿着锁的一方要按时续租，不能等测试线程空出来。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_client_beside_one_still_stopping_gets_its_own_store_holder() {
    let server = TestServer::start().await;
    let store = TempStore::create();
    let stuck = stuck_stop(&server, &store).await;
    let beside = MatrixSession::restore(&server.config("old", store.config()), stuck.saved)
        .await
        .expect("旧 Client 还在停时恢复会话");
    assert_ne!(holder(&beside), stuck.holder, "旧 Client 还活着，不能同名");
}

/// 挪到后台停的同步停完、旧 Client 也落了以后，存储租约跟着放：再建的 Client 接着用原来的名字，不白白重读（#1047）。
// 多线程：拿着锁的一方要按时续租，不能等测试线程空出来。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_sync_stopped_in_the_background_releases_its_store_holder() {
    let server = TestServer::start().await;
    let store = TempStore::create();
    let stuck = stuck_stop(&server, &store).await;
    drop(stuck.lock);
    assert!(
        gone(&stuck.client, Duration::from_secs(60)).await,
        "放开存储锁以后旧同步应停完、旧 Client 应释放"
    );
    // 后台任务隔一会儿才看一次 Client 落了没有。
    tokio::time::sleep(Duration::from_secs(3)).await;

    let next = MatrixSession::restore(&server.config("old", store.config()), stuck.saved)
        .await
        .expect("旧 Client 落了以后恢复会话");
    assert_eq!(holder(&next), stuck.holder);
}

/// 一个停不下来的旧会话：同存储上另一个 Client 拿着跨进程存储锁，旧会话的加密同步卡在拿锁上，停同步等不完。
struct StuckStop {
    saved: SavedSession,
    holder: String,
    client: WeakClient,
    lock: CrossProcessLockGuard,
    _other: MatrixSession,
}

async fn stuck_stop(server: &TestServer, store: &TempStore) -> StuckStop {
    let other = server
        .login_via(&server.url, &unique("lease"), "old", store.config())
        .await;
    let saved = other.saved().expect("登录状态");
    let lock = other
        .client()
        .encryption()
        .spin_lock_store(None)
        .await
        .expect("拿存储锁")
        .expect("SQLite 存储有跨进程锁");
    let old = MatrixSession::restore(&server.config("old", store.config()), saved.clone())
        .await
        .expect("恢复旧会话");
    // 房间列表不拿存储锁，照常同步；加密同步卡在拿锁上。
    old.start_sync().await.expect("旧会话同步");
    assert!(
        !old.stop_sync_within(Duration::from_millis(500)).await,
        "加密同步拿不到锁，停同步应等不完"
    );
    StuckStop {
        saved,
        holder: holder(&old),
        client: old.client().downgrade(),
        lock,
        _other: other,
    }
}

fn holder(session: &MatrixSession) -> String {
    match session.client().cross_process_lock_config() {
        CrossProcessLockConfig::MultiProcess { holder_name } => holder_name.clone(),
        CrossProcessLockConfig::SingleProcess => panic!("开在 SQLite 存储上的 Client 应有持有者名"),
    }
}
