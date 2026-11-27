//! 假 Tuwunel 的上传和下载。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::http::{Reply, auth, error, ok, reply};
use super::state::{Fake, SERVER};

pub(super) async fn upload(State(fake): State<Fake>, headers: HeaderMap, body: Bytes) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).map(|(user, _)| {
        let content_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        state.media.push((content_type, body.to_vec()));
        let id = state.next("media");
        state.media_uploaders.push(user);
        let index = state.media.len() - 1;
        state
            .media_ids
            .insert(format!("mxc://{SERVER}/{id}"), index);
        ok(json!({"content_uri": format!("mxc://{SERVER}/{id}")}))
    }))
}

pub(super) async fn download(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((server, id)): Path<(String, String)>,
) -> Response {
    let mut state = fake.state();
    let (user, _) = match auth(&state, &headers) {
        Ok(auth) => auth,
        Err(error) => return error.into_response(),
    };
    state.media_downloaders.push(user);
    let url = format!("mxc://{server}/{id}");
    let Some(index) = state.media_ids.get(&url) else {
        return error(StatusCode::NOT_FOUND, "M_NOT_FOUND").into_response();
    };
    let (content_type, data) = &state.media[*index];
    ([("content-type", content_type.clone())], data.clone()).into_response()
}
