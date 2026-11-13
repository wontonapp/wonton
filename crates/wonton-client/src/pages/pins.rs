//! Bot 钉在面板上的页面：从核心读、移除、标已看、收回往主面板钉页的同意。
//!
//! 页面存在服务器上，客户端每 5 秒对一次（和 Bot 活动一样不经推送）；连不上时留着上次的，不当成空。

use std::time::Duration;

use reqwest::{Client, Method, Response};
use url::Url;
use wonton_protocol::{
    BOT_PANEL_PINS_PATH, BotPanelPins, BotPinConsent, bot_panel_consent_path, bot_panel_pin_path,
    bot_panel_pin_seen_path,
};

use super::*;

const TIMEOUT: Duration = Duration::from_secs(30);

impl PagesLink {
    async fn pins_api(&self) -> Result<(Url, Client), CoreError> {
        let home = self.home.borrow().clone();
        let base = self
            .home_link
            .core_url()
            .ok_or_else(|| CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "panel-pins-not-connected"),
            })?;
        let proxy = home.proxy.ok_or_else(|| CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "panel-pins-not-connected"),
        })?;
        let client = Client::builder()
            .connect_timeout(TIMEOUT)
            .timeout(TIMEOUT)
            .proxy(
                reqwest::Proxy::all(crate::chat::proxy_url(&proxy))
                    .map_err(|e| invalid(&e.to_string()))?,
            )
            .build()
            .map_err(|e| invalid(&e.to_string()))?;
        Ok((base, client))
    }

    fn api_url(base: &Url, path: &str) -> Url {
        base.join(path).expect("fixed api path joins")
    }

    async fn expect(response: Response) -> Result<Response, CoreError> {
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let body = body.trim();
        let body = if body.is_empty() { "—" } else { body };
        Err(invalid(&l10n!(
            CATALOG,
            language(),
            "panel-pins-api-failed",
            status = status.to_string(),
            body = body
        )))
    }

    /// 服务器上的全部 Bot 钉页。
    pub(super) async fn fetch_bot_pins(&self) -> Result<BotPanelPins, CoreError> {
        let (base, client) = self.pins_api().await?;
        let response = client
            .request(Method::GET, Self::api_url(&base, BOT_PANEL_PINS_PATH))
            .send()
            .await
            .map_err(|e| invalid(&e.to_string()))?;
        let response = Self::expect(response).await?;
        let body = response.text().await.map_err(|e| invalid(&e.to_string()))?;
        serde_json::from_str(&body).map_err(|e| invalid(&e.to_string()))
    }

    pub(super) async fn remove_bot_pin(&self, id: &str) -> Result<(), CoreError> {
        let (base, client) = self.pins_api().await?;
        let response = client
            .request(
                Method::DELETE,
                Self::api_url(&base, &bot_panel_pin_path(id)),
            )
            .send()
            .await
            .map_err(|e| invalid(&e.to_string()))?;
        Self::expect(response).await.map(|_| ())
    }

    pub(super) async fn seen_bot_pin(&self, id: &str) -> Result<(), CoreError> {
        let (base, client) = self.pins_api().await?;
        let response = client
            .request(
                Method::POST,
                Self::api_url(&base, &bot_panel_pin_seen_path(id)),
            )
            .send()
            .await
            .map_err(|e| invalid(&e.to_string()))?;
        Self::expect(response).await.map(|_| ())
    }

    pub(super) async fn put_bot_consent(
        &self,
        plugin_id: &str,
        bot: &str,
        allowed: bool,
    ) -> Result<(), CoreError> {
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
            .map_err(|e| invalid(&e.to_string()))?;
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

    /// 拉一次，把视图需要的红点和同意跟着更新。失败记进 problem，保留原来的列表。
    ///
    /// 已知连不上家里的服务器时不拉，拉到一半才知道的也不记：连接提示已经说了连不上，这里再记一条
    /// 会在插件页的出错页上面再报一条“读不了页面设置”，把一件事报成两件。
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
                    let message = l10n!(
                        CATALOG,
                        language(),
                        "panel-pins-refresh-failed",
                        error = error.to_string()
                    );
                    let changed = state.pins_problem.as_deref() != Some(&message);
                    state.pins_problem = Some(message);
                    changed
                }
                Ok(pins) => {
                    let changed = state.pins_problem.is_some() || state.bot_pins != pins.pins;
                    state.pins_problem = None;
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
