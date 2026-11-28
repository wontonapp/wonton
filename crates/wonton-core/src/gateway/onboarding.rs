//! 引导状态与官方推荐清单接口。

use wonton_protocol::PermissionItem;

use std::collections::BTreeMap;
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
    DefaultBotPlugin, Onboarding, OnboardingNeed, OnboardingStep, OnboardingUpdate, StoreFailure,
    StoreItem, StoreProblem,
};

use super::{Gateway, TunnelPeer, api, permissions};
use crate::l10n;
use crate::plugin::InstallError;
use crate::plugin::languages::Languages;
use crate::plugin::store::{StoreError, create_private_dir, read_json, write_json};
use crate::plugin::storefront::onboarding::OnboardingCatalog;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredState {
    step: OnboardingStep,
}

impl Default for StoredState {
    fn default() -> Self {
        Self {
            step: OnboardingStep::Profile,
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
        langs: &Languages,
    ) -> Result<StoredState, InstallError> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_locked()?;
        if let Some(step) = update.step {
            // 中间的步骤只在 App 上（ADR-0038），这里只记从头开始和走完了。
            if !matches!(step, OnboardingStep::Profile | OnboardingStep::Finished) {
                return Err(InstallError::Invalid(l10n::text(
                    langs,
                    "gateway-onboarding-invalid-step",
                    &[("step", format!("{step:?}").into())],
                )));
            }
            state.step = step;
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
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ServerSettings)
    {
        return denied;
    }
    let person = match api::requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let langs = api::languages(&headers);
    let mut view = projection(&gateway, &langs).await;
    let state = gateway.onboarding.load().await;
    if let Err(denied) = api::current_permitted_requester(
        &gateway,
        peer,
        &headers,
        &person,
        PermissionItem::ServerSettings,
    ) {
        return *denied;
    }
    match state {
        Ok(state) => {
            view.step = state.step;
            if let Err(response) = super::bot_projection::onboarding(
                &gateway,
                peer.map(|Extension(peer)| peer),
                &headers,
                &person,
                &mut view,
            ) {
                return *response;
            }
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
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ServerSettings)
    {
        return denied;
    }
    let person = match api::requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
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
    let mut view = projection(&gateway, &langs).await;
    let state = gateway.onboarding.update(&update, &langs).await;
    if let Err(denied) = api::current_permitted_requester(
        &gateway,
        peer,
        &headers,
        &person,
        PermissionItem::ServerSettings,
    ) {
        return *denied;
    }
    let state = match state {
        Ok(state) => state,
        Err(error) => return api::install_error(&error),
    };
    if let Err(response) = super::bot_projection::onboarding(
        &gateway,
        peer.map(|Extension(peer)| peer),
        &headers,
        &person,
        &mut view,
    ) {
        return *response;
    }
    Json(Onboarding {
        step: state.step,
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
            details: l10n::text(langs, "gateway-onboarding-index-missing", &[]),
        })
    } else {
        catalog_problem
    };
    let (needs, prompts) = catalog
        .map(|catalog| catalog_content(catalog, &item_by_id, langs))
        .unwrap_or_default();
    Onboarding {
        step: OnboardingStep::Profile,
        default_bot_plugin: gateway
            .plugins
            .default_bot_plugin()
            .map(|id| DefaultBotPlugin {
                id: id.to_string(),
                bot_user_id: None,
                visibility: None,
            }),
        needs,
        prompts,
        catalog_problem,
        agent_intro: None,
    }
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

        let update = OnboardingUpdate {
            step: Some(OnboardingStep::Finished),
            agent_intro: None,
        };
        let saved = state.update(&update, &Languages::default()).await.unwrap();
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
