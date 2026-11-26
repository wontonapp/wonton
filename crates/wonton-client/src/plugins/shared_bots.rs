//! 当前请求人能发现的 Bot 列表（`GET SHARED_BOTS_PATH`）：走本地代理，核心凭隧道身份认出这台设备和它的人。
//!
//! 旧核心没有这个接口：404 当“没法用列表”返回 `Ok(None)`，客户端退回按活动快照猜的旧行为，不把它当失败。

use wonton_protocol::{CoreError, SHARED_BOTS_PATH, SharedBotsView};

use super::PluginsLink;
use super::api::{CoreApi, CoreApiError, READ_TIMEOUT, to_core_error};

impl PluginsLink {
    /// 取当前请求人的 Bot 可达清单；`Ok(None)` 表示核心还没有这个接口。
    pub(crate) async fn shared_bots(&self) -> Result<Option<SharedBotsView>, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        fetch_shared_bots(&api).await
    }
}

/// 404 是「这台核心还没这个接口」，回退到活动快照；其他失败才是真失败。
async fn fetch_shared_bots(api: &CoreApi) -> Result<Option<SharedBotsView>, CoreError> {
    match api
        .get_detailed::<SharedBotsView>(&shared_bots_segments(), &[], READ_TIMEOUT)
        .await
    {
        Ok(view) => Ok(Some(view)),
        // 旧核心的 axum 回 HTML 404，不一定会带 `x-wonton-error`；只看状态码。
        Err(CoreApiError::Http { status: 404, .. }) => Ok(None),
        Err(CoreApiError::Transport(problem)) => Err(to_core_error(problem)),
        Err(CoreApiError::Http {
            status, message, ..
        }) if status == 401 => Err(CoreError::Unauthorized { reason: message }),
        Err(CoreApiError::Http { message, .. }) => Err(CoreError::Upstream { reason: message }),
        Err(CoreApiError::Protocol(reason)) => Err(CoreError::Upstream { reason }),
    }
}

/// `SHARED_BOTS_PATH` 去掉 `/_wonton/api/` 之后的路径段，`CoreApi::get` 按段拼 URL。
fn shared_bots_segments() -> [&'static str; 1] {
    match SHARED_BOTS_PATH.strip_prefix("/_wonton/api/") {
        Some("bots") => ["bots"],
        _ => panic!("shared bots path must be one segment below /_wonton/api/"),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::{Value, json};
    use tokio::task::JoinHandle;

    use super::*;
    use crate::plugins::api::ServerVersion;

    async fn server() -> (CoreApi, JoinHandle<()>) {
        let router = Router::new().route(
            SHARED_BOTS_PATH,
            get(|| async { (StatusCode::NOT_FOUND, Json(json!({"error": "nope"}))) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let api = CoreApi {
            base: format!("http://{address}").parse().unwrap(),
            client: reqwest::Client::new(),
            languages: String::new(),
            version: ServerVersion::default(),
        };
        (api, task)
    }

    #[tokio::test]
    async fn a_404_is_an_old_core_not_a_failure() {
        let (api, _server) = server().await;
        assert!(fetch_shared_bots(&api).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_valid_body_is_parsed_into_rows() {
        let router = Router::new().route(
            SHARED_BOTS_PATH,
            get(|| async {
                Json(json!({
                    "state": "ready",
                    "bots": [{
                        "bot": {"plugin_id": "app.bot", "bot": "default"},
                        "user_id": "@bot:test",
                        "title": "Bot",
                        "icon": null,
                        "access": {"state": "own"}
                    }]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let api = CoreApi {
            base: format!("http://{address}").parse().unwrap(),
            client: reqwest::Client::new(),
            languages: String::new(),
            version: ServerVersion::default(),
        };
        let view = fetch_shared_bots(&api).await.unwrap().unwrap();
        assert_eq!(view.bots.len(), 1);
        assert_eq!(view.bots[0].title, "Bot");
        let _: Value = json!(view.state);
    }
}
