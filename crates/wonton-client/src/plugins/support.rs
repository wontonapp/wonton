//! 跟家里的连接、图片缓存文件名、原子写和锁。都只在这棵模块树里用。

use super::*;

/// 跟着家里的连接：连上（App 启动、重新配对、断线恢复）时取列表，壳在页面出现时发的刷新可能比隧道早；换了服务器时忘掉上一台
/// 的东西。
pub(super) async fn follow_home(link: Weak<PluginsLink>) {
    let Some(mut home) = link.upgrade().map(|link| link.home.subscribe()) else {
        return;
    };
    let mut connected = false;
    let (mut was_unpaired, mut was_disconnected) = {
        let status = home.borrow().status.clone();
        (
            status == HomeStatus::NotConfigured,
            matches!(status, HomeStatus::Disconnected { .. }),
        )
    };
    let mut server = home.borrow().server_id.clone();
    loop {
        let (now_connected, now_unpaired, disconnected, unreachable, next_server) = {
            let view = home.borrow_and_update();
            (
                view.status == HomeStatus::Connected,
                view.status == HomeStatus::NotConfigured,
                matches!(view.status, HomeStatus::Disconnected { .. }),
                matches!(
                    view.status,
                    HomeStatus::Disconnected {
                        reason: wonton_protocol::HomeFailure::TransportUnavailable,
                        ..
                    }
                ),
                view.server_id.clone(),
            )
        };
        let Some(link) = link.upgrade() else { return };
        if server.is_some() && next_server != server {
            link.forget_server();
        }
        server = next_server;
        // 离开“还没配对”、或者掉成“连不上”时，让商店跟着换状态。被拒绝
        // （Unauthorized）不在这里重取：重取会触发一次重拨，“被拒绝后不自动重连”就破了。
        let left_unpaired = was_unpaired && !now_unpaired && !now_connected;
        let unreachable_now = unreachable && !was_disconnected;
        was_unpaired = now_unpaired;
        was_disconnected = disconnected;
        if (left_unpaired || unreachable_now) && link.store_wanted.load(Ordering::Relaxed) {
            link.refresh_store(false);
        }
        if !now_connected && connected {
            link.view.send_if_modified(|view| {
                let changed = view.installed.as_mut().is_some_and(identity::invalidate);
                if changed {
                    PluginsLink::derive(view);
                }
                changed
            });
        }
        if disconnected {
            link.forget_storage();
        }
        if now_connected && !connected {
            link.refresh_installed();
            link.fetch_core_upgrade();
            if link.store_wanted.load(Ordering::Relaxed) {
                link.refresh_store(false);
            }
        }
        connected = now_connected;
        drop(link);
        if home.changed().await.is_err() {
            return;
        }
    }
}

/// `/_wonton/api/images/<sha256>/<文件名>` 换成一个文件名。
pub(super) fn cache_name(path: &str) -> String {
    path.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) async fn save_atomically(
    dir: &Path,
    target: &Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    // 同一张图可能同时在加载（主屏和详情页都要），各写各的临时文件。
    static NEXT: AtomicU64 = AtomicU64::new(0);
    tokio::fs::create_dir_all(dir).await?;
    let partial =
        target.with_extension(format!("{}.partial", NEXT.fetch_add(1, Ordering::Relaxed)));
    tokio::fs::write(&partial, bytes).await?;
    tokio::fs::rename(&partial, target).await
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
