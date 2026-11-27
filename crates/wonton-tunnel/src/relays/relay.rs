use std::fmt;
use std::sync::Arc;

use iroh::{RelayConfig, RelayMap, RelayUrl};
use iroh_relay::RelayQuicConfig;
use serde::{Deserialize, Serialize};

use super::url::wire_url_text;

/// 一台中继：地址、可选的准入令牌、可选的 QUIC 端口、档位（数字小的优先）。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relay {
    pub url: RelayUrl,
    pub token: Option<RelayToken>,
    pub quic_port: Option<u16>,
    pub tier: u8,
}

impl Relay {
    /// iroh 认识的中继配置。自建中继按 ADR-0048 的部署约定开 QUIC 地址发现（UDP 7842），和 iroh
    /// 对 n0 中继的默认一样。
    pub fn relay_config(&self) -> RelayConfig {
        let mut config = RelayConfig::from(self.url.clone());
        if let Some(token) = &self.token {
            config = config.with_auth_token(token.expose());
        }
        config
    }

    /// 校验规则和 [`RelayView::validate`] 相同；内部配置解析出来的项也要过。
    pub fn validate(&self) -> Result<(), String> {
        self.view().validate()
    }

    /// 报给 App 的写法：令牌单独放，地址里永远不带。
    pub fn view(&self) -> RelayView {
        RelayView {
            url: self.url.clone(),
            token: self.token.clone(),
            quic_port: self.quic_port,
            tier: self.tier,
        }
    }
}

/// 报给 App 的一台中继（ADR-0048）：配对码和 `/_wonton/api/tunnel` 用这个写法。令牌只放在
/// `token` 里、不进地址，所以地址可以原样写进日志和界面；两个下发通道都只给已认证的设备，
/// 插件网页拿不到。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayView {
    pub url: RelayUrl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<RelayToken>,
    /// QUIC 地址发现的 UDP 端口；空着用 iroh 的默认端口（[`iroh::defaults::DEFAULT_RELAY_QUIC_PORT`]）。
    ///
    /// 老配对码、连接信息里没有这个字段，缺的按默认端口处理，所以加了 `serde(default)`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quic_port: Option<u16>,
    pub tier: u8,
}

impl RelayView {
    /// 从连接信息或配对码读回来的表项要过这一关：地址里不许藏令牌（令牌单独放），令牌不能是空的，
    /// 而且得是 https，令牌不走明文。
    pub fn validate(&self) -> Result<(), String> {
        if self.url.scheme() != "https" {
            return Err(format!("{}：中继地址要用 https", wire_url_text(&self.url)));
        }
        if !self.url.username().is_empty() || self.url.password().is_some() {
            return Err(format!(
                "{}：地址里不能带 userinfo，令牌要单独放",
                wire_url_text(&self.url)
            ));
        }
        if let Some(token) = &self.token
            && token.expose().is_empty()
        {
            return Err(format!("{}：令牌不能是空的", wire_url_text(&self.url)));
        }
        if self.quic_port == Some(0) {
            return Err(format!("{}：QUIC 端口不能是 0", wire_url_text(&self.url)));
        }
        Ok(())
    }

    /// iroh 认识的中继配置。自建中继按 ADR-0048 的部署约定开 QUIC 地址发现（UDP 7842），和 iroh
    /// 对 n0 中继的默认一样。
    pub fn relay_config(&self) -> RelayConfig {
        let mut config = RelayConfig::from(self.url.clone());
        if let Some(port) = self.quic_port {
            config.quic = Some(RelayQuicConfig::new(port));
        }
        if let Some(token) = &self.token {
            config = config.with_auth_token(token.expose());
        }
        config
    }
}

impl fmt::Debug for Relay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Relay")
            .field("url", &self.url)
            .field("token", &self.token)
            .field("quic_port", &self.quic_port)
            .field("tier", &self.tier)
            .finish()
    }
}

impl From<RelayView> for Relay {
    fn from(view: RelayView) -> Self {
        Self {
            url: view.url,
            token: view.token,
            quic_port: view.quic_port,
            tier: view.tier,
        }
    }
}

/// 中继的准入令牌。`Debug` 不打印原文，打进日志不会泄露。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayToken(String);

impl RelayToken {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// 原文。
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RelayToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// 从报给 App 的表项建 iroh 的 [`RelayMap`]：只放前 `tiers` 档。
pub fn relay_map(relays: &[RelayView], tiers: usize) -> RelayMap {
    relays
        .iter()
        .filter(|relay| (relay.tier as usize) < tiers)
        .map(|relay| Arc::new(relay.relay_config()))
        .collect()
}
