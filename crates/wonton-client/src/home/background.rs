//! 后台策略（ADR-0032 的前后台与 ADR-0003 保持连接的例外）：App 和网页媒体的前后台状态怎么决定探测和
//! 自动重连；保持连接时壳的前台服务让核心在后台照常工作。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Instant;

use tokio::sync::watch;
use wonton_protocol::{HomeStatus, HomeViewModel, PushMethod};

use super::{HomeLink, MEDIA_RELEASE_GRACE};

impl HomeLink {
    /// `AppForegrounded`：在后台检查连接，必要时重连。
    pub(crate) fn foregrounded(self: &Arc<Self>) {
        self.app_backgrounded.store(false, Ordering::SeqCst);
        self.sync_background();
        self.lock_tracker().foregrounded(Instant::now());
        self.recount();
        self.runtime.spawn(self.clone().resume());
    }

    pub(crate) fn stop_periodic_checks(&self) {
        self.periodic_checks.send_replace(false);
    }

    /// `AppBackgrounded`：停止探测和自动重连；壳报告过在放声音时不真的停（见 [`Self::page_media_active`]）。
    /// 连接不主动断，后台音乐和正在传的文件还在用它。
    pub(crate) fn backgrounded(&self) {
        self.app_backgrounded.store(true, Ordering::SeqCst);
        self.sync_background();
    }

    /// 网页媒体开始或停止播放：播放期间 App 退到后台也照常探测、自动重连，隧道不会空闲断开。
    /// 停止后由核心再保活 [`MEDIA_RELEASE_GRACE`]，只留一瞬的换曲间隙也用这里盖住。
    pub(crate) fn page_media_active(self: &Arc<Self>, active: bool) {
        let mut pending = self
            .media_release
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(release) = pending.take() {
            release.abort();
        }
        let generation = self.media_generation.fetch_add(1, Ordering::SeqCst) + 1;
        if active {
            if self.page_media.swap(true, Ordering::SeqCst) {
                return;
            }
            self.sync_background();
            self.lock_tracker().foregrounded(Instant::now());
            self.recount();
            self.runtime.spawn(self.clone().resume());
        } else if self.page_media.load(Ordering::SeqCst) {
            // 宽限期内保持 page_media 为真：这时切后台仍是前台保活，不会停探测。
            let home = self.clone();
            let release = self.runtime.spawn(async move {
                tokio::time::sleep(MEDIA_RELEASE_GRACE).await;
                let _pending = home
                    .media_release
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if home.media_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                home.page_media.store(false, Ordering::SeqCst);
                home.sync_background();
            });
            *pending = Some(release);
        }
    }

    fn sync_background(&self) {
        // 保持连接（ADR-0003）下壳的前台服务让隧道一直活着，核心不算“在后台”：探测和自动重连照常。
        let background = self.app_backgrounded.load(Ordering::SeqCst)
            && !self.page_media.load(Ordering::SeqCst)
            && !self.keep_connection.load(Ordering::SeqCst);
        if *self.background.borrow() != background {
            self.background.send_replace(background);
            // 真的进了后台（没在放声音、没开保持连接）才停取地址：保活期间和前台一样。
            if background {
                self.stop_address_refresh();
            }
        }
    }
}

/// 已配对（有连接信息）、收新消息的方式是保持连接时，`keep_connection` 为真：Android 的壳照它启停
/// “保持连接”的前台服务。连接状态和方式任何一个变了都重算。
pub(super) async fn follow_keep_connection(
    home: Weak<HomeLink>,
    mut home_rx: watch::Receiver<HomeViewModel>,
    mut method: watch::Receiver<PushMethod>,
) {
    loop {
        let keep = {
            let view = home_rx.borrow_and_update();
            let paired = !matches!(view.status, HomeStatus::NotConfigured);
            paired && *method.borrow() == PushMethod::Keepalive
        };
        if let Some(link) = home.upgrade() {
            link.view.send_if_modified(|view| {
                let changed = view.keep_connection != keep;
                view.keep_connection = keep;
                changed
            });
            // 壳照 ViewModel 的 keep_connection 启停前台服务；核心这边照同一个值不停探测和自动重连。
            let changed = link.keep_connection.swap(keep, Ordering::SeqCst) != keep;
            if changed {
                link.sync_background();
            }
        }
        tokio::select! {
            changed = home_rx.changed() => if changed.is_err() { return },
            changed = method.changed() => if changed.is_err() { return },
        }
    }
}
