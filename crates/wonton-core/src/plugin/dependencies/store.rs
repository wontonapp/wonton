use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::PluginId;
use crate::plugin::InstallError;
use crate::plugin::store::{read_json, write_json};

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Connection {
    pub consumer: String,
    pub provider: String,
    pub capability: String,
    pub key: Option<String>,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub rebuild: bool,
    pub problem: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HandoffConsent {
    pub from: wonton_protocol::BotRef,
    pub target: wonton_protocol::BotRef,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct State {
    pub defaults: BTreeMap<String, String>,
    pub selected: BTreeMap<String, BTreeMap<String, String>>,
    pub providers: Vec<String>,
    pub connections: Vec<Connection>,
    /// 提供方令牌（ADR-0061 第 3 节），键 `<能力>/<插件 ID>`，见 [`super::provider_token`]。
    #[serde(default)]
    pub provider_tokens: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handoff_consents: Vec<HandoffConsent>,
}

impl State {
    /// 一个提供方在这项能力上的令牌。
    pub(super) fn provider_token(&self, capability: &str, provider: &PluginId) -> Option<&String> {
        self.provider_tokens
            .get(&format!("{capability}/{}", provider.as_str()))
    }
}

pub(crate) struct Dependencies {
    path: PathBuf,
    state: RwLock<Result<State, String>>,
    pub changing: tokio::sync::Mutex<()>,
    #[cfg(test)]
    pub routes: RwLock<BTreeMap<(String, String), crate::plugin::upstreams::Endpoint>>,
}

impl Dependencies {
    pub fn open(path: Result<PathBuf, crate::plugin::store::StoreError>) -> Self {
        let (path, state) = match path {
            Ok(path) => {
                let state = read_json(&path)
                    .map(Option::unwrap_or_default)
                    .map_err(|e| e.to_string());
                (path, state)
            }
            Err(error) => (PathBuf::new(), Err(error.to_string())),
        };
        Self {
            path,
            state: RwLock::new(state),
            changing: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            routes: RwLock::new(BTreeMap::new()),
        }
    }

    pub(super) fn load(&self) -> Result<State, InstallError> {
        self.state
            .read()
            .expect("能力记录锁中毒")
            .as_ref()
            .cloned()
            .map_err(|e| InstallError::Storage(e.clone()))
    }
    pub(super) fn inspect<T>(&self, view: impl FnOnce(&State) -> T) -> Result<T, InstallError> {
        let guard = self.state.read().expect("能力记录锁中毒");
        guard
            .as_ref()
            .map(view)
            .map_err(|e| InstallError::Storage(e.clone()))
    }

    pub(super) fn change(&self, update: impl FnOnce(&mut State)) -> Result<(), InstallError> {
        let mut guard = self.state.write().expect("能力记录锁中毒");
        let mut next = guard
            .as_ref()
            .map_err(|e| InstallError::Storage(e.clone()))?
            .clone();
        let before = next.clone();
        update(&mut next);
        if next == before {
            return Ok(());
        }
        write_json(&self.path, &next)?;
        *guard = Ok(next);
        Ok(())
    }
}
