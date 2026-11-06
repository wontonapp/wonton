//! 引导状态与官方推荐清单接口。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use hyper::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use wonton_protocol::{
    AgentLogin, Onboarding, OnboardingAgent, OnboardingNeed, OnboardingStep, OnboardingUpdate,
    PluginKind, StoreFailure, StoreItem, StoreProblem,
};

use super::{Gateway, TunnelPeer, api};
use crate::plugin::InstallError;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::PluginId;
use crate::plugin::store::{StoreError, create_private_dir, read_json, write_json};
use crate::plugin::storefront::index::Index;
use crate::plugin::storefront::onboarding::OnboardingCatalog;
use crate::plugin::views::agent_login_options;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredState {
    step: OnboardingStep,
    primary_agent: Option<String>,
    replaced_agent_asked: bool,
}

impl Default for StoredState {
    fn default() -> Self {
        Self {
            step: OnboardingStep::Profile,
            primary_agent: None,
            replaced_agent_asked: false,
        }
    }
}

/// 引导状态的私有持久化；同一进程内的读改写串行。
pub(crate) struct OnboardingState {
    path: PathBuf,
    lock: Mutex<()>,
}

impl OnboardingState {
    pub(crate) fn new(data_dir: PathBuf) -> Self {
        Self {
            path: data_dir.join("core").join("onboarding.json"),
            lock: Mutex::new(()),
        }
    }

    async fn load(&self) -> Result<StoredState, InstallError> {
        let _guard = self.lock.lock().await;
        self.load_locked()
    }

    fn load_locked(&self) -> Result<StoredState, InstallError> {
        read_json(&self.path)
            .map_err(storage)
            .map(|state| state.unwrap_or_default())
    }

    async fn update(
        &self,
        update: &OnboardingUpdate,
        agent_ids: &BTreeSet<String>,
    ) -> Result<StoredState, InstallError> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_locked()?;
        if let Some(step) = update.step {
            // 中间的步骤只在 App 上（ADR-0038），这里只记从头开始和走完了。
            if !matches!(step, OnboardingStep::Profile | OnboardingStep::Finished) {
                return Err(InstallError::Invalid(format!(
                    "step 只能是 profile 或 finished，不是 {step:?}"
                )));
            }
            state.step = step;
        }
        if let Some(primary) = &update.primary_agent {
            if !agent_ids.contains(primary) {
                return Err(InstallError::Invalid(format!(
                    "primary_agent 不是可选 Agent：{primary}"
                )));
            }
            state.primary_agent = Some(primary.clone());
        }
        if let Some(asked) = update.replaced_agent_asked {
            state.replaced_agent_asked = asked;
        }
        create_private_dir(self.path.parent().expect("onboarding state parent"))
            .map_err(storage)?;
        write_json(&self.path, &state).map_err(storage)?;
        Ok(state)
    }
}

fn storage(error: StoreError) -> InstallError {
    InstallError::Storage(error.to_string())
}

pub(crate) async fn get(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = api::plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = api::languages(&headers);
    let mut view = projection(&gateway, &langs).await;
    match gateway.onboarding.load().await {
        Ok(state) => {
            view.step = state.step;
            view.primary_agent = state.primary_agent;
            view.replaced_agent_asked = state.replaced_agent_asked;
            Json(view).into_response()
        }
        Err(error) => api::install_error(&error),
    }
}

pub(crate) async fn put(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = api::plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let update: OnboardingUpdate = match serde_json::from_slice(&body) {
        Ok(update) => update,
        Err(error) => {
            return api::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &error.to_string(),
            );
        }
    };
    let langs = api::languages(&headers);
    let view = projection(&gateway, &langs).await;
    let agent_ids = view.agents.iter().map(|agent| agent.id.clone()).collect();
    let state = match gateway.onboarding.update(&update, &agent_ids).await {
        Ok(state) => state,
        Err(error) => return api::install_error(&error),
    };
    Json(Onboarding {
        step: state.step,
        primary_agent: state.primary_agent,
        replaced_agent_asked: state.replaced_agent_asked,
        ..view
    })
    .into_response()
}

async fn projection(gateway: &Gateway, langs: &Languages) -> Onboarding {
    let (index, catalog_problem, items) = gateway.plugins.onboarding_store_listing(langs).await;
    let item_by_id: BTreeMap<_, _> = items
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect();
    let installed = gateway.plugins.installed(langs).plugins;
    let installed_by_id: BTreeMap<_, _> = installed
        .into_iter()
        .map(|plugin| (plugin.id.clone(), plugin))
        .collect();

    let catalog_problem = match index.onboarding_problem.as_ref() {
        Some(details) => Some(StoreProblem {
            reason: StoreFailure::InvalidIndex,
            details: details.clone(),
        }),
        None => catalog_problem,
    };
    let catalog = index.onboarding.as_ref();
    let catalog_problem = if catalog.is_none() && catalog_problem.is_none() {
        Some(StoreProblem {
            reason: StoreFailure::InvalidIndex,
            details: "官方索引没有 onboarding 清单".to_owned(),
        })
    } else {
        catalog_problem
    };
    let agents = onboarding_agents(
        gateway,
        langs,
        &index,
        catalog,
        &item_by_id,
        &installed_by_id,
    );

    let (needs, prompts) = catalog
        .map(|catalog| catalog_content(catalog, &item_by_id, langs))
        .unwrap_or_default();
    Onboarding {
        step: OnboardingStep::Profile,
        primary_agent: None,
        replaced_agent_asked: false,
        agents,
        needs,
        prompts,
        catalog_problem,
    }
}

fn onboarding_agents(
    gateway: &Gateway,
    langs: &Languages,
    index: &Index,
    catalog: Option<&OnboardingCatalog>,
    items: &BTreeMap<String, StoreItem>,
    installed: &BTreeMap<String, wonton_protocol::InstalledPlugin>,
) -> Vec<OnboardingAgent> {
    let mut agents = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(catalog) = catalog {
        for id in &catalog.agents {
            let Some(item) = items.get(id) else {
                tracing::warn!(plugin = %id, "官方 onboarding agent 不在插件清单中，跳过");
                continue;
            };
            if item.kind != PluginKind::Agent {
                tracing::warn!(plugin = %id, "官方 onboarding agent 不是 agent，跳过");
                continue;
            }
            let installed = installed.get(id);
            if installed.is_some_and(|plugin| plugin.kind != PluginKind::Agent) {
                tracing::warn!(plugin = %id, "官方 onboarding agent 不是已安装的 agent，跳过");
                continue;
            }
            if !seen.insert(id.clone()) {
                continue;
            }
            let logins = installed
                .map(|_| installed_logins(gateway, langs, id))
                .unwrap_or_else(|| catalog_agent_logins(index, id, langs));
            agents.push(match installed {
                Some(plugin) => agent_with_logins(item, plugin, logins),
                None => agent(item, logins),
            });
        }
    }
    for plugin in installed.values() {
        if plugin.kind != PluginKind::Agent || !seen.insert(plugin.id.clone()) {
            continue;
        }
        let logins = installed_logins(gateway, langs, &plugin.id);
        let agent = match items.get(&plugin.id) {
            Some(item) => agent_with_logins(item, plugin, logins),
            None => OnboardingAgent {
                id: plugin.id.clone(),
                title: plugin.title.clone(),
                source: None,
                status: Some(plugin.status.clone()),
                bot_user_id: plugin.bot_user_id.clone(),
                logins,
                blocked: None,
            },
        };
        agents.push(agent);
    }
    agents
}

fn catalog_agent_logins(index: &Index, id: &str, langs: &Languages) -> Vec<AgentLogin> {
    index
        .plugins
        .iter()
        .find(|listed| listed.manifest.id.as_str() == id)
        .and_then(|listed| listed.manifest.agent.as_ref())
        .map(|agent| agent_login_options(agent, langs))
        .unwrap_or_default()
}
fn installed_logins(gateway: &Gateway, langs: &Languages, id: &str) -> Vec<AgentLogin> {
    PluginId::parse(id)
        .ok()
        .map(|id| gateway.plugins.agent_logins(&id, langs))
        .unwrap_or_default()
}

fn catalog_content(
    catalog: &OnboardingCatalog,
    items: &BTreeMap<String, StoreItem>,
    langs: &Languages,
) -> (Vec<OnboardingNeed>, Vec<String>) {
    let needs = catalog
        .needs
        .iter()
        .map(|need| OnboardingNeed {
            id: need.id.clone(),
            title: langs.pick_owned(&need.title),
            plugins: need
                .plugins
                .iter()
                .filter_map(|id| {
                    let item = items.get(id);
                    if item.is_none() {
                        tracing::warn!(plugin = %id, "官方 onboarding need 插件不在清单中，跳过");
                    }
                    item.cloned()
                })
                .collect(),
        })
        .collect();
    let prompts = catalog
        .prompts
        .iter()
        .map(|prompt| langs.pick_owned(prompt))
        .collect();
    (needs, prompts)
}

fn agent(item: &StoreItem, logins: Vec<AgentLogin>) -> OnboardingAgent {
    OnboardingAgent {
        id: item.id.clone(),
        title: item.title.clone(),
        source: Some(item.source.clone()),
        status: None,
        bot_user_id: None,
        logins,
        blocked: item.blocked.clone(),
    }
}

fn agent_with_logins(
    item: &StoreItem,
    installed: &wonton_protocol::InstalledPlugin,
    logins: Vec<AgentLogin>,
) -> OnboardingAgent {
    OnboardingAgent {
        id: item.id.clone(),
        title: item.title.clone(),
        source: Some(item.source.clone()),
        status: Some(installed.status.clone()),
        bot_user_id: installed.bot_user_id.clone(),
        logins,
        blocked: item.blocked.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn missing_state_has_initial_values_and_updates_atomically() {
        let dir = tempdir().unwrap();
        let state = OnboardingState::new(dir.path().to_owned());
        let initial = state.load().await.unwrap();
        assert_eq!(initial.step, OnboardingStep::Profile);
        assert_eq!(initial.primary_agent, None);
        assert!(!initial.replaced_agent_asked);

        let update = OnboardingUpdate {
            step: Some(OnboardingStep::Finished),
            primary_agent: Some("com.example.agent".to_owned()),
            replaced_agent_asked: Some(true),
        };
        let saved = state
            .update(
                &update,
                &["com.example.agent".to_owned()].into_iter().collect(),
            )
            .await
            .unwrap();
        assert_eq!(saved.step, OnboardingStep::Finished);
        assert_eq!(state.load().await.unwrap(), saved);
    }

    #[tokio::test]
    async fn corrupt_state_is_storage_error_and_is_not_overwritten() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("core/onboarding.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json").unwrap();
        let state = OnboardingState::new(dir.path().to_owned());
        let error = state.load().await.unwrap_err();
        assert!(matches!(error, InstallError::Storage(_)));
        assert_eq!(std::fs::read(&path).unwrap(), b"not json");
    }
}
