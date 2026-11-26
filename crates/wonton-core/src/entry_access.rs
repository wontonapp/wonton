//! 公网访问总开关和它的三道闸门（ADR-0090 第 2 节）：默认关，只有主人在 App 里能开。
//!
//! 开关落盘在 `core/entry-access.json`（目录 0700、文件 0600，原子写）。关掉时按顺序撤掉连接器、
//! 清空入口公网前端的证书列表，并在 8484 上拒绝公网标记；局域网入口和经 iroh 的配对不受影响。
//! 三道各自的结果留给状态接口如实报告，没关干净不能当已关。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;
use wonton_protocol::{GateState, PublicAccess, PublicSwitchState};

use crate::entry_containers::EntryContainers;
use crate::entry_settings::EntrySettings;
use crate::plugin::store::{StoreError, create_private_dir, read_json, write_json};

/// 当前开关和上一次执行闸门的结果。写入串行，读不阻塞。
pub struct EntryAccess {
    path: PathBuf,
    state: watch::Sender<PublicAccess>,
    /// 和 `Gateway::public_access` 共用同一个标志：持久开关一改，热路径上的判断立刻跟着变。
    flag: Arc<AtomicBool>,
    gates: Mutex<PublicSwitchState>,
    /// 一次开关变更从写盘到闸门做完都要占着它，周期对账也用它。
    apply: tokio::sync::Mutex<()>,
}

impl EntryAccess {
    /// 读盘上的开关；文件不在时按默认关。读不出来是错误，不拿默认值盖掉损坏的文件。
    pub fn open(path: PathBuf) -> Result<Self, StoreError> {
        let stored = read_json::<PublicAccess>(&path)?.unwrap_or(PublicAccess { enabled: false });
        let (state, _) = watch::channel(stored);
        Ok(Self {
            path,
            state,
            flag: Arc::new(AtomicBool::new(stored.enabled)),
            gates: Mutex::new(PublicSwitchState {
                connector: GateState::Unknown,
                crt_list: GateState::Unknown,
                core: GateState::Unknown,
            }),
            apply: tokio::sync::Mutex::new(()),
        })
    }

    pub fn enabled(&self) -> bool {
        self.state.borrow().enabled
    }

    pub fn public_access_flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }

    pub fn get(&self) -> PublicAccess {
        *self.state.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<PublicAccess> {
        self.state.subscribe()
    }

    pub(crate) async fn apply_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.apply.lock().await
    }

    pub fn gates(&self) -> PublicSwitchState {
        self.lock_gates().clone()
    }

    pub fn record_gates(&self, gates: PublicSwitchState) {
        *self.lock_gates() = gates;
    }

    /// 先落盘再改内存：写不进去时不改状态，调用方如实报错。
    pub fn save(&self, access: PublicAccess) -> Result<(), StoreError> {
        create_private_dir(self.path.parent().expect("开关文件都有目录"))?;
        write_json(&self.path, &access)?;
        self.state.send_replace(access);
        self.flag.store(access.enabled, Ordering::Relaxed);
        Ok(())
    }

    fn lock_gates(&self) -> MutexGuard<'_, PublicSwitchState> {
        self.gates.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 开关和入口运行时的接线：网关、状态接口、配对状态和周期对账共用一份。
pub struct EntryAdmin {
    pub access: std::sync::Arc<EntryAccess>,
    containers: Option<std::sync::Arc<EntryContainers>>,
    install_dir: PathBuf,
}

impl EntryAdmin {
    pub fn new(
        access: std::sync::Arc<EntryAccess>,
        containers: Option<std::sync::Arc<EntryContainers>>,
        install_dir: PathBuf,
    ) -> Self {
        Self {
            access,
            containers,
            install_dir,
        }
    }

    pub fn core_in_container(&self) -> bool {
        self.containers
            .as_ref()
            .is_some_and(|c| c.core_in_container())
    }

    pub fn lan_forward_available(&self) -> bool {
        self.containers
            .as_ref()
            .is_some_and(|containers| containers.lan_forward_available())
    }

    pub fn containers(&self) -> Option<&std::sync::Arc<EntryContainers>> {
        self.containers.as_ref()
    }

    pub fn install_dir(&self) -> &Path {
        &self.install_dir
    }

    pub fn settings(&self) -> anyhow::Result<Option<EntrySettings>> {
        EntrySettings::load(&self.install_dir)
    }

    /// 下发开关：同一个值重复下发不动入口。先写盘，再逐道闸门做，结果记下来。
    pub async fn apply(&self, enabled: bool) -> Result<PublicSwitchState, StoreError> {
        let _guard = self.access.apply_lock().await;
        if self.access.enabled() == enabled {
            return Ok(self.access.gates());
        }
        self.access.save(PublicAccess { enabled })?;
        let settings = EntrySettings::load(&self.install_dir);
        let gates = match (&self.containers, settings) {
            (Some(containers), Ok(settings)) => {
                containers
                    .apply_public_access(settings.as_ref(), enabled)
                    .await
            }
            (Some(containers), Err(error)) => {
                // 设置读不了就做不了容器和证书列表；核心那道闸门只是代码，照旧成立。
                containers.report_switch(&error.to_string())
            }
            (None, Ok(_)) | (None, Err(_)) => PublicSwitchState {
                connector: GateState::Failed {
                    detail: "核心没跑在容器里，连接器起不来".to_owned(),
                },
                crt_list: GateState::Failed {
                    detail: "核心没跑在容器里，入口容器不在".to_owned(),
                },
                core: GateState::Done,
            },
        };
        self.access.record_gates(gates.clone());
        Ok(gates)
    }

    /// 按持久开关对账入口：客户端开关的重算和启动都用这一份开关值。
    pub async fn reconcile(
        &self,
        settings: Option<&EntrySettings>,
    ) -> Result<(), crate::entry_containers::EntryError> {
        match &self.containers {
            Some(containers) => containers.reconcile(settings, self.access.enabled()).await,
            None => Err(crate::entry_containers::EntryError::CoreNotInContainer),
        }
    }

    /// 客户端开关写入口模型走这里：容器实例只有一份，没有入口容器时如实报错。
    pub async fn apply_clients_model(
        &self,
        settings: &EntrySettings,
        public_access: bool,
        public_labels: Vec<String>,
        lan_labels: Vec<String>,
    ) -> Result<(), crate::entry_containers::EntryError> {
        match &self.containers {
            Some(containers) => {
                containers
                    .apply_clients_model(settings, public_access, public_labels, lan_labels)
                    .await
            }
            None => Err(crate::entry_containers::EntryError::CoreNotInContainer),
        }
    }
}

#[cfg(test)]
#[path = "entry_access_tests.rs"]
mod entry_access_tests;
