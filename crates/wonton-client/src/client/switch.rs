//! 换服务器（ADR-0042）：改服务器列表、停掉旧作用域、按新槽位建一个，把新作用域的 ViewModel 接到壳订阅的发送端上。

use std::sync::{Arc, Mutex, MutexGuard};

use wonton_protocol::{CoreError, DeviceNetwork, PushEndpoint};

use super::ClientCore;
use crate::files::FileOpener;
use crate::scope::Scope;
use crate::servers::{Change, ServerList};

/// 壳只在启动或变化时发一次的输入，新作用域要再收到一遍。
#[derive(Default)]
pub(super) struct Replay {
    pub(super) languages: Option<Vec<String>>,
    pub(super) network: Option<DeviceNetwork>,
    pub(super) push: Option<PushInput>,
    pub(super) opener: Option<Arc<dyn FileOpener>>,
    pub(super) no_periodic_checks: bool,
}

pub(super) enum PushInput {
    Endpoint(PushEndpoint),
    Unavailable(String),
}

impl ClientCore {
    /// Pairing always starts in an unpaired scope, including direct/debug scans and re-pairing.
    pub(super) fn pair_home(&self, text: &str, device_name: &str) -> Result<(), CoreError> {
        let _switching = lock(&self.switching);
        let Some(code) = self.scope().home.validate_pairing(text, device_name)? else {
            return Ok(());
        };
        if self.scope().home.view().server_id.is_some() {
            self.change_servers_locked(ServerList::add)?;
        }
        self.scope().home.pair(code, device_name);
        Ok(())
    }
}

impl crate::matrix_login::SlotSwitch for ClientCore {
    fn add_matrix_slot(
        &self,
        slot: &str,
        account: wonton_protocol::MatrixAccount,
    ) -> Result<(), CoreError> {
        self.change_servers(|servers| servers.add_matrix(slot, account))
    }
}

impl ClientCore {
    pub(super) fn scope(&self) -> Arc<Scope> {
        self.scope
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn replay(&self) -> MutexGuard<'_, Replay> {
        lock(&self.replay)
    }

    /// 服务器列表的操作。要换服务器的，先停掉旧作用域再建新的，见 [`Scope::open`]；新的建不起来（存储出错）时
    /// 重新打开旧的那台，如实返回错误。
    pub(super) fn change_servers(
        &self,
        change: impl FnOnce(&ServerList) -> Result<Change, CoreError>,
    ) -> Result<(), CoreError> {
        let _switching = lock(&self.switching);
        self.change_servers_locked(change)
    }

    fn change_servers_locked(
        &self,
        change: impl FnOnce(&ServerList) -> Result<Change, CoreError>,
    ) -> Result<(), CoreError> {
        let change = change(&self.servers)?;
        let previous = self.scope();
        if let Some(slot) = change.switch_to.filter(|slot| *slot != previous.slot) {
            previous.close(true);
            self.visits.end(&slot);
            match self.open_scope(&slot) {
                Ok(next) => self.install(next),
                Err(error) => {
                    tracing::warn!(%error, slot, "could not open the other server, staying on the current one");
                    self.install(self.open_scope(&previous.slot)?);
                    return Err(error);
                }
            }
        }
        for slot in &change.discard {
            self.visits.end(slot);
        }
        self.servers.discard(&change.discard)
    }

    fn install(&self, scope: Arc<Scope>) {
        *self
            .scope
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = scope.clone();
        self.attach(&scope);
    }

    fn open_scope(&self, slot: &str) -> Result<Arc<Scope>, CoreError> {
        // feature 开时 views() 返回读锁，关时直接返回引用；这里统一拿成 &Views。
        #[cfg(feature = "ui-fake")]
        let views = &*self.views();
        #[cfg(not(feature = "ui-fake"))]
        let views = self.views();
        Scope::open(
            slot,
            &self.root,
            &self.servers.slot_dir(slot),
            self.network,
            self.servers.subscribe(),
            Some(views),
        )
    }

    /// 重放壳只发一次的输入，并跟着作用域的配对结果更新服务器列表。
    pub(super) fn attach(&self, scope: &Scope) {
        self.replay_into(scope);
        self.follow_pairing(scope);
    }

    fn replay_into(&self, scope: &Scope) {
        let replay = self.replay();
        if let Some(languages) = &replay.languages {
            scope.set_languages(languages);
        }
        if let Some(network) = replay.network {
            scope.home.network_changed(network);
        }
        match &replay.push {
            Some(PushInput::Endpoint(endpoint)) => {
                // 壳发来时已经校验过。
                let _ = scope.push.set_endpoint(endpoint.clone());
            }
            Some(PushInput::Unavailable(message)) => scope.push.unavailable(message.clone()),
            None => {}
        }
        if let Some(opener) = &replay.opener {
            scope.files.set_opener(opener.clone());
        }
        if replay.no_periodic_checks {
            scope.home.stop_periodic_checks();
            scope.plugins.stop_upgrade_polling();
        }
    }

    /// 这个槽位连上服务器后记下它的 ID；同一台以前配在别的槽位上的，删掉旧槽位。
    fn follow_pairing(&self, scope: &Scope) {
        let mut home = scope.home.subscribe();
        let servers = self.servers.clone();
        let slot = scope.slot.clone();
        scope.spawn(async move {
            loop {
                let server_id = home.borrow_and_update().server_id.clone();
                if let Some(server_id) = server_id {
                    let paired = servers
                        .paired(&slot, &server_id)
                        .and_then(|change| servers.discard(&change.discard));
                    if let Err(error) = paired {
                        tracing::warn!(%error, "could not save the configured servers");
                    }
                }
                if home.changed().await.is_err() {
                    return;
                }
            }
        });
    }
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
