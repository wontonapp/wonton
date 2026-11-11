use crate::{config::Config, dns::Dns, store::Store};
use axum::{
    Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;

pub struct App {
    pub config: Config,
    pub store: Mutex<Store>,
    pub dns: Dns,
    pub client: reqwest::Client,
    pub runtime: Mutex<Runtime>,
}

#[derive(Default)]
pub struct Runtime {
    pub proxies: BTreeMap<String, String>,
    pub proxy_reservations: BTreeMap<String, u64>,
    pub connections: BTreeMap<String, u64>,
    pub metrics_at: u64,
}

pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "invalid signed request".into())
    }
    pub fn forbidden() -> Self {
        Self(
            StatusCode::FORBIDDEN,
            "home unavailable or not owned".into(),
        )
    }
    pub fn bad(reason: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, reason.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        tracing::error!(error = %error, "home operation failed");
        Self(
            StatusCode::BAD_GATEWAY,
            "upstream or persistence operation failed".into(),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":{"kind":self.1}}))).into_response()
    }
}

impl App {
    pub fn new(config: Config, store: Store) -> anyhow::Result<Arc<Self>> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Arc::new(Self {
            dns: Dns::new(config.clone(), client.clone()),
            config,
            store: Mutex::new(store),
            client,
            runtime: Mutex::new(Runtime::default()),
        }))
    }

    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/health", get(|| async { StatusCode::OK }))
            .route("/homes", post(crate::api::register))
            .route(
                "/homes/{home}",
                get(crate::api::status).delete(crate::api::delete),
            )
            .route("/homes/{home}/caa", axum::routing::put(crate::api::caa))
            .route("/acme/update", post(crate::api::acme_update))
            .layer(axum::extract::DefaultBodyLimit::max(16384))
            .with_state(self.clone())
    }

    pub fn hooks(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/frp", post(crate::hooks::handle))
            .layer(axum::extract::DefaultBodyLimit::max(16384))
            .with_state(self.clone())
    }

    pub async fn reconcile(&self) -> anyhow::Result<()> {
        let mut store = self.store.lock().await;
        let clock = crate::store::now();
        for home in store.data.homes.values_mut() {
            if let Some(pending) = home
                .pending_caa
                .as_ref()
                .filter(|caa| caa.effective_at <= clock)
            {
                home.caa = Some(pending.clone());
                home.pending_caa = None;
            }
        }
        // Persist desired state before DNS. On crash/restart every desired record is reconciled.
        store.save()?;
        for (label, home) in &store.data.homes {
            self.dns.reconcile(label, home).await?;
        }
        Ok(())
    }
}
