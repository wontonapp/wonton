//! Bot 钉在面板上的页面：从核心读、移除、标已看、收回往主面板钉页的同意。
//!
//! 页面存在服务器上，客户端每 5 秒对一次（和 Bot 活动一样不经推送）；连不上时留着上次的，不当成空。

use std::borrow::Cow;
use std::time::Duration;

use reqwest::{Client, Method, Response};
use url::Url;
use wonton_protocol::{
    BOT_PANEL_PINS_PATH, BotPanelPins, BotPinConsent, PagesFailure, PagesProblem,
    bot_panel_consent_path, bot_panel_pin_path, bot_panel_pin_seen_path,
};
use wonton_tunnel::{CATEGORY_HEADER, ERROR_HEADER};

use super::*;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(serde::Deserialize)]
struct ErrorEnvelope<'a> {
    #[serde(borrow)]
    error: ErrorKind<'a>,
}

#[derive(serde::Deserialize)]
struct ErrorKind<'a> {
    #[serde(borrow)]
    kind: Cow<'a, str>,
}

fn page_problem(reason: PagesFailure, message: impl ToString) -> PagesProblem {
    PagesProblem {
        reason: Some(reason),
        message: message.to_string(),
    }
}

fn classify_http_failure(
    status: u16,
    tunnel_error: Option<&str>,
    category: Option<&str>,
    body: &str,
) -> PagesFailure {
    let body_kind = serde_json::from_str::<ErrorEnvelope<'_>>(body)
        .ok()
        .map(|body| body.error.kind);
    match tunnel_error.or(category).or(body_kind.as_deref()) {
        Some("unauthorized" | "unauthenticated" | "proxy-unauthorized") => {
            PagesFailure::Unauthorized
        }
        Some("tunnel-unavailable" | "transport-unavailable" | "closed") => {
            PagesFailure::TransportUnavailable
        }
        Some("upstream-unavailable" | "storage") => PagesFailure::Upstream,
        Some("cross_site" | "permission_denied" | "permission_required") => PagesFailure::Forbidden,
        Some("protocol") => PagesFailure::Protocol,
        Some("not_found") => PagesFailure::NotFound,
        Some("invalid_input") => PagesFailure::InvalidInput,
        Some(_) => PagesFailure::Other,
        None => match status {
            400 => PagesFailure::InvalidInput,
            401 => PagesFailure::Unauthorized,
            403 => PagesFailure::Forbidden,
            404 => PagesFailure::NotFound,
            _ => PagesFailure::Upstream,
        },
    }
}

fn pin_action_confirmed(action: &PinAction, pins: &BotPanelPins) -> bool {
    match action {
        PinAction::Remove(id) => !pins.pins.iter().any(|pin| pin.id == *id),
        PinAction::Consent {
            plugin_id,
            bot,
            allowed,
        } => {
            let current = pins
                .consents
                .iter()
                .find(|entry| entry.plugin_id == *plugin_id && entry.bot == *bot);
            current.map_or(!allowed, |entry| entry.main_consent == *allowed)
        }
    }
}

impl PagesLink {
    async fn pins_api(&self) -> Result<(Url, Client), PagesProblem> {
        let home = self.home.borrow().clone();
        let not_ready = || {
            page_problem(
                PagesFailure::NotReady,
                l10n!(CATALOG, language(), "panel-pins-not-connected"),
            )
        };
        let base = self.home_link.core_url().ok_or_else(not_ready)?;
        let proxy = home.proxy.ok_or_else(not_ready)?;
        let client = Client::builder()
            .connect_timeout(TIMEOUT)
            .timeout(TIMEOUT)
            .proxy(
                reqwest::Proxy::all(crate::chat::proxy_url(&proxy))
                    .map_err(|error| page_problem(PagesFailure::InvalidInput, error))?,
            )
            .build()
            .map_err(|error| page_problem(PagesFailure::InvalidInput, error))?;
        Ok((base, client))
    }

    fn api_url(base: &Url, path: &str) -> Url {
        base.join(path).expect("fixed api path joins")
    }

    async fn expect(response: Response) -> Result<Response, PagesProblem> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let tunnel_error = response
            .headers()
            .get(ERROR_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let category = response
            .headers()
            .get(CATEGORY_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .text()
            .await
            .map_err(|error| page_problem(PagesFailure::TransportUnavailable, error))?;
        let body = body.trim();
        let detail = if body.is_empty() {
            tunnel_error
                .as_deref()
                .or(category.as_deref())
                .unwrap_or("—")
        } else {
            body
        };
        let reason = classify_http_failure(
            status.as_u16(),
            tunnel_error.as_deref(),
            category.as_deref(),
            body,
        );
        Err(page_problem(
            reason,
            l10n!(
                CATALOG,
                language(),
                "panel-pins-api-failed",
                status = status.to_string(),
                body = detail
            ),
        ))
    }

    /// 服务器上的全部 Bot 钉页。
    pub(super) async fn fetch_bot_pins(&self) -> Result<BotPanelPins, PagesProblem> {
        let (base, client) = self.pins_api().await?;
        let response = client
            .request(Method::GET, Self::api_url(&base, BOT_PANEL_PINS_PATH))
            .send()
            .await
            .map_err(|error| page_problem(PagesFailure::TransportUnavailable, error))?;
        let response = Self::expect(response).await?;
        let body = response
            .text()
            .await
            .map_err(|error| page_problem(PagesFailure::TransportUnavailable, error))?;
        serde_json::from_str(&body)
            .map_err(|error| page_problem(PagesFailure::InvalidResponse, error))
    }

    pub(super) async fn remove_bot_pin(&self, id: &str) -> Result<(), PagesProblem> {
        let (base, client) = self.pins_api().await?;
        let response = client
            .request(
                Method::DELETE,
                Self::api_url(&base, &bot_panel_pin_path(id)),
            )
            .send()
            .await
            .map_err(|error| page_problem(PagesFailure::TransportUnavailable, error))?;
        Self::expect(response).await.map(|_| ())
    }

    pub(super) async fn seen_bot_pin(&self, id: &str) -> Result<(), PagesProblem> {
        let (base, client) = self.pins_api().await?;
        let response = client
            .request(
                Method::POST,
                Self::api_url(&base, &bot_panel_pin_seen_path(id)),
            )
            .send()
            .await
            .map_err(|error| page_problem(PagesFailure::TransportUnavailable, error))?;
        Self::expect(response).await.map(|_| ())
    }

    pub(super) async fn put_bot_consent(
        &self,
        plugin_id: &str,
        bot: &str,
        allowed: bool,
    ) -> Result<(), PagesProblem> {
        let (base, client) = self.pins_api().await?;
        let response = client
            .request(
                Method::PUT,
                Self::api_url(&base, &bot_panel_consent_path(plugin_id, bot)),
            )
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&BotPinConsent { allowed }).expect("consent serializes"))
            .send()
            .await
            .map_err(|error| page_problem(PagesFailure::TransportUnavailable, error))?;
        Self::expect(response).await?;
        let mut state = lock(&self.state);
        if let Some(entry) = state
            .bot_pin_consents
            .iter_mut()
            .find(|entry| entry.plugin_id == plugin_id && entry.bot == bot)
        {
            entry.main_consent = allowed;
        }
        Ok(())
    }

    /// 拉一次，把视图需要的红点和同意跟着更新。失败记进问题，保留原来的列表。
    ///
    /// 已知连不上家里的服务器时不拉，拉到一半才知道的也不记：连接提示已经说了连不上，
    /// 这里再记一条会把一件事报成两件。
    pub(super) async fn refresh_bot_pins(self: &Arc<Self>) {
        if self.home_link.tunnel_outage().is_some() {
            return;
        }
        let result = self.fetch_bot_pins().await;
        if result.is_err() && self.home_link.tunnel_outage().is_some() {
            return;
        }
        let changed = {
            let mut state = lock(&self.state);
            match result {
                Err(error) => {
                    let problem = PagesProblem {
                        reason: error.reason,
                        message: l10n!(
                            CATALOG,
                            language(),
                            "panel-pins-refresh-failed",
                            error = error.message
                        ),
                    };
                    let changed = state.pins_problem.as_ref() != Some(&problem);
                    state.pins_problem = Some(problem);
                    changed
                }
                Ok(pins) => {
                    let action_cleared = state
                        .action_problem
                        .as_ref()
                        .is_some_and(|(action, _)| pin_action_confirmed(action, &pins));
                    let changed = state.pins_problem.is_some()
                        || state.bot_pins != pins.pins
                        || state.bot_pin_consents != pins.consents
                        || action_cleared;
                    state.pins_problem = None;
                    if action_cleared {
                        state.action_problem = None;
                    }
                    state.bot_pins = pins.pins;
                    state.bot_pin_consents = pins.consents;
                    changed
                }
            }
        };
        if changed {
            self.recompute();
        }
    }
}
