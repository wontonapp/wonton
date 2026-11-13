//! 为另一台服务器临时开的作用域（ADR-0042）：推送不切换当前服务器。通知回复按需打开持久待发队列，
//! 不打开底栏和引导；最后一个使用者归还后关闭运行时。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;

use tokio::sync::watch;
use wonton_protocol::CoreError;

use super::ScopeRuntime;
use crate::app_lock::AppLockLink;
use crate::chat::ChatLink;
use crate::composer::{ComposerLink, uplink::LiveUplink};
use crate::files::FilesLink;
use crate::home::{HomeLink, Network, Storage};
use crate::lock_screen::LockScreenLink;
use crate::plugins::PluginsLink;
use crate::push::PushLink;
use crate::push_method::PushMethodLink;
use crate::servers::ServerList;

pub(crate) struct Visit {
    pub(crate) slot: String,
    pub(crate) chat: Arc<ChatLink>,
    pub(crate) push: Arc<PushLink>,
    pub(crate) runtime: ScopeRuntime,
    home: Arc<HomeLink>,
    dir: PathBuf,
    composer: OnceLock<Arc<ComposerLink>>,
}

impl Visit {
    fn open(
        slot: &str,
        root: &Path,
        dir: &Path,
        network: Network,
        app_lock: &AppLockLink,
    ) -> Result<Arc<Self>, CoreError> {
        let runtime = ScopeRuntime::new();
        let handle = runtime.handle().expect("the runtime was just built");
        let storage = Storage::open(root, dir)?;
        // 和正式作用域一致：方式是整机设置，放设备级的 root（见 Scope::open）。
        let push_method = PushMethodLink::new(root);
        let lock_screen = LockScreenLink::new(root)?;
        let home = HomeLink::new(
            handle.clone(),
            crate::sharing::hub::bind(handle.clone())?,
            storage,
            network,
            None,
            push_method.subscribe(),
        )?;
        // 聊天部件要插件部件来认 Bot；Bot 的轮询随下面的 backgrounded 停掉，插件部件只在连上时取一次列表。
        let plugins = PluginsLink::new(handle.clone(), home.clone(), dir, None, false);
        // 临时作用域不补取：卡片成功什么都不做。开着测试钩子时决定是个 future，直接完成。
        let card_success: crate::chat::CardSuccess = {
            #[cfg(not(feature = "test-hooks"))]
            let noop: crate::chat::CardSuccess = Arc::new(|_: &str| {});
            #[cfg(feature = "test-hooks")]
            let noop: crate::chat::CardSuccess = Arc::new(|_: &str| Box::pin(async {}));
            noop
        };
        let chat = ChatLink::new(
            handle.clone(),
            home.clone(),
            slot,
            dir,
            card_success,
            plugins,
            crate::chat::ChatChannels {
                view: None,
                confirmations: None,
                method: push_method.subscribe(),
                app_lock: app_lock.clone(),
                lock_screen: lock_screen.subscribe(),
                session_out: watch::channel(None).0,
                room_panel_out: watch::channel(None).0,
                external: false,
            },
        )?;
        chat.backgrounded();
        let push = PushLink::new(
            handle,
            home.clone(),
            chat.clone(),
            dir,
            push_method.subscribe(),
            lock_screen.subscribe(),
        );
        Ok(Arc::new(Self {
            slot: slot.to_owned(),
            chat,
            push,
            runtime,
            home,
            dir: dir.to_owned(),
            composer: OnceLock::new(),
        }))
    }

    pub(crate) fn composer(&self) -> Result<Arc<ComposerLink>, CoreError> {
        let handle = self.runtime.handle().ok_or_else(|| CoreError::NotReady {
            reason: wonton_l10n::l10n!(
                crate::l10n::CATALOG,
                crate::l10n::language(),
                "scope-switched-server"
            ),
        })?;
        Ok(self
            .composer
            .get_or_init(|| {
                let _entered = handle.enter();
                let files = FilesLink::new(
                    handle.clone(),
                    self.home.clone(),
                    self.chat.clone(),
                    &self.dir,
                    None,
                );
                ComposerLink::new(
                    handle.clone(),
                    Arc::new(LiveUplink {
                        chat: self.chat.clone(),
                        files,
                        home: self.home.clone(),
                    }),
                    &self.dir,
                )
            })
            .clone())
    }
}

/// 开着的临时作用域，按槽位。同时要用同一台的共用一个，最后一个用完就关掉。换到这台、移除这台之前先
/// [`Visits::end`]：两个作用域不能同时用一个槽位的 Matrix 库。
pub(crate) struct Visits {
    servers: Arc<ServerList>,
    root: PathBuf,
    network: Network,
    /// 和换服务器共用的锁：不会在切到那台的同时为它开临时作用域。
    switching: Arc<Mutex<()>>,
    /// App 锁：临时作用域弹出的通知也要照着它藏内容。
    app_lock: AppLockLink,
    entries: Mutex<HashMap<String, Entry>>,
}

enum Entry {
    Open {
        visit: Arc<Visit>,
        users: usize,
    },
    /// 最后一个用完了，运行时在这个线程上停。
    Stopping(JoinHandle<()>),
}

/// 用着一个临时作用域；丢掉时归还，壳中途放弃了推送也一样。
pub(crate) struct Entered {
    visits: Arc<Visits>,
    pub(crate) visit: Arc<Visit>,
}

impl Drop for Entered {
    fn drop(&mut self) {
        self.visits.leave(&self.visit);
    }
}

impl Visits {
    pub(crate) fn new(
        servers: Arc<ServerList>,
        root: PathBuf,
        network: Network,
        switching: Arc<Mutex<()>>,
        app_lock: AppLockLink,
    ) -> Arc<Self> {
        Arc::new(Self {
            servers,
            root,
            network,
            switching,
            app_lock,
            entries: Mutex::default(),
        })
    }

    /// 这个槽位不是当前这台时，拿到它的临时作用域；是当前这台时为空。
    pub(crate) fn other(self: &Arc<Self>, slot: &str) -> Result<Option<Entered>, CoreError> {
        let _switching = lock(&self.switching);
        if self.servers.current() == slot {
            return Ok(None);
        }
        self.enter(slot).map(Some)
    }

    /// 拿到这个槽位的临时作用域，没有就开一个。调用方拿着换服务器的锁，确认过它不是当前这台。
    pub(crate) fn enter(self: &Arc<Self>, slot: &str) -> Result<Entered, CoreError> {
        let mut entries = lock(&self.entries);
        if let Some(Entry::Open { visit, users }) = entries.get_mut(slot) {
            *users += 1;
            return Ok(self.entered(visit.clone()));
        }
        if let Some(Entry::Stopping(stopping)) = entries.remove(slot) {
            join(stopping);
        }
        let dir = self.servers.slot_dir(slot);
        let visit = Visit::open(slot, &self.root, &dir, self.network, &self.app_lock)?;
        entries.insert(
            slot.to_owned(),
            Entry::Open {
                visit: visit.clone(),
                users: 1,
            },
        );
        Ok(self.entered(visit))
    }

    fn entered(self: &Arc<Self>, visit: Arc<Visit>) -> Entered {
        Entered {
            visits: self.clone(),
            visit,
        }
    }

    fn leave(&self, visit: &Arc<Visit>) {
        let mut entries = lock(&self.entries);
        let Some(Entry::Open { visit: open, users }) = entries.get_mut(&visit.slot) else {
            return;
        };
        // 中途被 `end` 关掉、又开了一个新的：不是同一个，不动它。
        if !Arc::ptr_eq(open, visit) {
            return;
        }
        *users -= 1;
        if *users > 0 {
            return;
        }
        entries.remove(&visit.slot);
        if let Some(stopping) = visit.runtime.stopping() {
            entries.insert(visit.slot.clone(), Entry::Stopping(stopping));
        }
    }

    /// 关掉这个槽位上的临时作用域，等它停下。正在用它的报“已经切到另一台服务器”。
    pub(crate) fn end(&self, slot: &str) {
        let entry = lock(&self.entries).remove(slot);
        match entry {
            Some(Entry::Open { visit, .. }) => visit.runtime.stop(),
            Some(Entry::Stopping(stopping)) => join(stopping),
            None => {}
        }
    }
}

fn join(stopping: JoinHandle<()>) {
    if stopping.join().is_err() {
        tracing::warn!("the temporary scope's runtime did not shut down cleanly");
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
