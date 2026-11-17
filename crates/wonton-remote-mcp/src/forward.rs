use std::{io, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;

use crate::{
    Connector, failure,
    headers::{RemoteRequest, response_headers},
    legacy::{self, Opening, Session},
    network_failure,
};

pub(crate) async fn forward(State(connector): State<Arc<Connector>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let context = match RemoteRequest::parse(&parts.headers) {
        Ok(context) => context,
        Err(detail) => return failure(StatusCode::BAD_REQUEST, "invalid_input", detail),
    };
    if context.auth == "oauth" {
        return failure(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            "OAuth is not implemented by this connector version",
        );
    }
    if !matches!(parts.method, Method::GET | Method::POST | Method::DELETE) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let session_id = context
        .headers
        .get("mcp-session-id")
        .and_then(|id| id.to_str().ok());
    if let Some(id) = session_id.filter(|id| id.starts_with("wonton-legacy-")) {
        let Some(session) = connector.legacy.find(id, &context) else {
            return failure(
                StatusCode::NOT_FOUND,
                "remote_error",
                "Unknown legacy connector session",
            );
        };
        return match parts.method {
            Method::GET => session.events(),
            Method::DELETE => {
                connector.legacy.remove(id);
                StatusCode::NO_CONTENT.into_response()
            }
            Method::POST => match to_bytes(body, usize::MAX).await {
                Ok(bytes) => {
                    session
                        .post(&connector.network, context.headers, bytes, None)
                        .await
                }
                Err(_) => failure(
                    StatusCode::BAD_REQUEST,
                    "invalid_input",
                    "Could not read MCP request body",
                ),
            },
            _ => unreachable!(),
        };
    }
    // Only the initialize exchange can require replay into an HTTP+SSE message endpoint.
    let initializing = parts.method == Method::POST && session_id.is_none();
    if initializing {
        return initialize(&connector, context, body).await;
    }
    let body = reqwest::Body::wrap_stream(
        body.into_data_stream()
            .map_err(|_| io::Error::other("MCP request body interrupted")),
    );
    match connector
        .network
        .send(parts.method, &context.url, context.headers, Some(body))
        .await
    {
        Ok(remote) => response(remote),
        Err(error) => network_failure(error),
    }
}

async fn initialize(connector: &Connector, context: RemoteRequest, body: Body) -> Response {
    let opening = connector.legacy.begin(&context.plugin);
    let cancellation = opening.cancellation();
    tokio::select! {
        biased;
        _ = legacy::cancelled(cancellation) => legacy::expired(),
        response = initialize_inner(connector, context, body, opening) => response,
    }
}

async fn initialize_inner(
    connector: &Connector,
    context: RemoteRequest,
    body: Body,
    opening: Opening,
) -> Response {
    let bytes = match to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return failure(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                "Could not read MCP request body",
            );
        }
    };
    let remote = match connector
        .network
        .send(
            Method::POST,
            &context.url,
            context.headers.clone(),
            Some(bytes.clone().into()),
        )
        .await
    {
        Ok(remote) => remote,
        Err(error) => return network_failure(error),
    };
    let try_legacy = matches!(
        remote.status(),
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) && serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .is_some_and(|message| {
            message.get("method").and_then(|value| value.as_str()) == Some("initialize")
        });
    if !try_legacy {
        return response(remote);
    }
    let session = match Session::open(&connector.network, &context, &opening).await {
        Ok(Some(session)) => session,
        Ok(None) => return response(remote),
        Err(error) => return error.into_response(),
    };
    session
        .post(&connector.network, context.headers, bytes, Some(opening))
        .await
}

pub(crate) fn response(remote: reqwest::Response) -> Response {
    let status = remote.status();
    if status.is_redirection() {
        return failure(
            StatusCode::BAD_GATEWAY,
            "remote_error",
            "Remote MCP redirects are not supported",
        );
    }
    let headers = response_headers(remote.headers());
    let stream = remote
        .bytes_stream()
        .map_err(|_| io::Error::other("Remote MCP response stream interrupted"));
    let mut response = Body::from_stream(stream).into_response();
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
