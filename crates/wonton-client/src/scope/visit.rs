//! 为另一台服务器临时开的作用域（ADR-0042 的“推送带上服务器 ID”）：推送来自不是当前这台的服务器、推送地址变了
//! 要给那台重新登记时，只开连接、聊天和推送三个部件（聊天要的插件部件不轮询），用完就关掉。底栏、文件这些不开：它们会
//! 轮询、会接着传文件。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use tokio::sync::watch;
use wonton_protocol::CoreError;

use super::ScopeRuntime;
use crate::chat::ChatLink;
use crate::home::{HomeLink, Network, Storage};
use crate::plugins::PluginsLink;
use crate::push::PushLink;
use crate::push_method::PushMethodLink;
use crate::servers::ServerList;

pub(crate) struct Visit {
    pub(crate) slot: String,
    pub(crate) chat: Arc<ChatLink>,
    pub(crate) push: Arc<PushLink>,
    pub(crate) runtime: ScopeRuntime,
}

impl Visit {
    fn open(slot: &str, root: &Path, dir: &Path, network: Network) -> Result<Arc<Self>, CoreError> {
        let runtime = ScopeRuntime::new();
        let handle = runtime.handle().expect("the runtime was just built");
        let storage = Storage::open(root, dir)?;
        // 和正式作用域一致：方式是整机设置，放设备级的 root（见 Scope::open）。
        let push_method = PushMethodLink::new(root);
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
                method: push_method.subscribe(),
                session_out: watch::channel(None).0,
                room_panel_out: watch::channel(None).0,
                external: false,
            },
        )?;
        chat.backgrounded();
        let push = PushLink::new(handle, home, chat.clone(), dir, push_method.subscribe());
        Ok(Arc::new(Self {
            slot: slot.to_owned(),
            chat,
            push,
            runtime,
        }))
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
    ) -> Arc<Self> {
        Arc::new(Self {
            servers,
            root,
            network,
            switching,
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
        let visit = Visit::open(slot, &self.root, &dir, self.network)?;
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
