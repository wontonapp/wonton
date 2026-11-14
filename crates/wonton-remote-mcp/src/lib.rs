mod config;
mod forward;
mod headers;
mod legacy;
mod network;
mod probe;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, delete, get, post},
};

pub use config::{AddressPolicy, Config, ResolveFuture, Resolver};
pub use network::NetworkError;

struct Connector {
    network: network::Network,
    legacy: legacy::Sessions,
}

/// In-process entry point; relaxing address policy requires an explicit library argument.
pub fn router(config: Config) -> Router {
    Router::new()
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/mcp", any(forward::forward))
        .route("/_wonton/probe", post(probe_request))
        .route("/_wonton/plugins/{id}", delete(forget))
        .with_state(Arc::new(Connector {
            network: network::Network::new(config),
            legacy: legacy::Sessions::default(),
        }))
}

async fn probe_request(
    State(connector): State<Arc<Connector>>,
    Json(input): Json<probe::ProbeRequest>,
) -> Response {
    if input.auth == "oauth" {
        return failure(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            "OAuth is not implemented by this connector version",
        );
    }
    Json(probe::probe(&connector.network, input).await).into_response()
}

async fn forget(State(connector): State<Arc<Connector>>, Path(id): Path<String>) -> StatusCode {
    connector.legacy.forget(&id);
    StatusCode::NO_CONTENT
}

fn failure(status: StatusCode, outcome: &str, detail: &str) -> Response {
    let mut response = (
        status,
        Json(serde_json::json!({ "outcome": outcome, "detail": detail })),
    )
        .into_response();
    if let Ok(value) = outcome.parse() {
        response
            .headers_mut()
            .insert("x-wonton-remote-status", value);
    }
    response
}

fn network_failure(error: NetworkError) -> Response {
    let status = match error {
        NetworkError::BlockedAddress | NetworkError::InvalidInput => StatusCode::BAD_REQUEST,
        NetworkError::Unreachable | NetworkError::ProxyUnavailable => StatusCode::BAD_GATEWAY,
    };
    failure(status, error.outcome(), error.detail())
}
