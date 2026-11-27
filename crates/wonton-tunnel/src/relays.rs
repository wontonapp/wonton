//! 中继表：一张带档位和准入令牌的中继列表（ADR-0048）。
//!
//! 核心的来源是环境变量 [`RELAYS_VAR`]：档位之间用 `;` 分开，前面的档优先；同一档里的中继用 `,`
//! 分开；令牌写在地址的 userinfo 里（`https://TOKEN@relay.example.org`），解析后从地址里去掉；
//! 关键字 `n0` 是 iroh 内置的 n0 公共中继那一组。不设时等于 `n0`。核心把整张表（连同令牌）放进
//! `/_wonton/api/tunnel` 和配对码，App 用它建自己一侧的 endpoint。
//!
//! 分档回退（[`spawn_tier_fallback`]）：endpoint 一开始只放最优先的一档，这一档全部连不上、持续
//! 超过 [`FALLBACK_AFTER`] 才把下一档加进来；被撤的档里有中继真的收我们（探针 endpoint 带令牌
//! 握手成功）才撤回来。档内谁快用谁由 iroh 按探测延迟挑（家中继），这里不排先后。
//!
//! 中继拒绝（令牌被吊销、配额用完）和连不上走同一条重试路径：iroh 对它连不上的中继按 10 毫秒起、
//! 每次翻倍、封顶 16 秒的退避重连。这一层不再自己加节奏——改 endpoint 的中继表会让 iroh 跑一遍完整
//! net_report（每台中继 3 个 HTTPS 探测），所以探针只用单台中继的 map，档位不变时一次都不动。

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use anyhow::Context;
use iroh::endpoint::default_relay_mode;
use iroh::endpoint::presets;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey, Watcher};
use iroh_relay::RelayQuicConfig;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use url::Url;

/// 读中继表的环境变量。
pub const RELAYS_VAR: &str = "WONTON_RELAYS";

/// 关键字：iroh 内置的 n0 公共中继那一组。
const N0: &str = "n0";

/// 最优先一档全部连不上持续多久才用下一档。
///
/// 30 秒：iroh 连中继是带退避的重试，中继重启、网络抖动一般几十秒内自己恢复，不值得马上把更远的
/// 档拉进来；再长的话，中继真下线时用户等得太久（ADR-0048 里已接受切档前会有一段连不上）。
pub const FALLBACK_AFTER: Duration = Duration::from_secs(30);

/// 回退判定多久看一次 iroh 的家中继状态。阈值是秒级的，1 秒的粒度够了。
const POLL_INTERVAL: Duration = Duration::from_secs(1);

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
    fn relay_config(&self) -> RelayConfig {
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
        if !self.url.username().is_empty() || self.url.password().is_some() {
            return Err(format!(
                "{}：地址里不能带 userinfo，令牌要单独放",
                relay_url_text(&self.url)
            ));
        }
        if self.url.scheme() != "https" {
            return Err(format!("{}：中继地址要用 https", relay_url_text(&self.url)));
        }
        if let Some(token) = &self.token
            && token.expose().is_empty()
        {
            return Err(format!("{}：令牌不能是空的", relay_url_text(&self.url)));
        }
        if self.quic_port == Some(0) {
            return Err(format!("{}：QUIC 端口不能是 0", relay_url_text(&self.url)));
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

/// 中继表：按档位排好的中继，前面的档优先。同一地址只出现一次，后面的项盖过前面的（ADR-0048）。
///
/// `remove` 是覆盖层的删除标记（`!https://…`）：[`overlay`](Self::overlay) 先把这些地址从底表删掉，
/// 再放自己这一层的项。主人配置里有内置项想删掉时用。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayTable {
    relays: Vec<Relay>,
    remove: Vec<RelayUrl>,
}

impl RelayTable {
    /// 不设 [`RELAYS_VAR`] 时的表：n0 的公共中继一档。
    pub fn builtin_n0() -> Self {
        Self {
            relays: n0_relays(0).collect(),
            remove: Vec::new(),
        }
    }

    /// 从配置解析出来的表项组表；地址规范成 [`canonical_url`] 的一种写法。
    pub fn from_relays(relays: Vec<Relay>) -> Self {
        let mut relays: Vec<_> = relays.into_iter().map(canonical_relay).collect();
        relays.sort_by_key(|relay| relay.tier);
        Self {
            relays,
            remove: Vec::new(),
        }
    }

    /// 从连接信息或配对码里带来的表项组表。表项要先用 [`RelayView::validate`] 校验过。
    pub fn from_views(relays: Vec<RelayView>) -> Self {
        Self::from_relays(relays.into_iter().map(Relay::from).collect())
    }

    /// 记录要删掉的地址。合并时先删，再放自己的项。
    pub fn with_remove(mut self, remove: Vec<RelayUrl>) -> Self {
        for url in remove {
            let url = canonical_url(url);
            if !self.remove.contains(&url) {
                self.remove.push(url);
            }
        }
        self
    }

    /// 要删掉的地址。
    pub fn remove(&self) -> &[RelayUrl] {
        &self.remove
    }

    /// 解析 [`RELAYS_VAR`] 的值。写得不合法时说清是哪一项、为什么，不静默跳过。
    ///
    /// `!https://relay.example.org` 是删除标记：把这一项从更早的来源里删掉。`#7843` 是 QUIC 端口。
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut relays: Vec<Relay> = Vec::new();
        let mut remove: Vec<RelayUrl> = Vec::new();
        for (index, tier_text) in text.split(';').enumerate() {
            let tier = u8::try_from(index).map_err(|_| "档位太多".to_owned())?;
            for item in tier_text.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    return Err(format!("第 {} 档里有空的中继项", index + 1));
                }
                if item == N0 {
                    relays.extend(n0_relays(tier));
                } else if let Some(url) = item.strip_prefix('!') {
                    if url == N0 {
                        remove.extend(n0_relays(0).map(|relay| relay.url));
                    } else {
                        remove.push(relay_from_item(url, tier)?.url);
                    }
                } else {
                    relays.push(relay_from_item(item, tier)?);
                }
            }
        }
        // 同一地址后面的项盖过前面的：档位和令牌以最后一次写的为准。
        let mut unique: Vec<Relay> = Vec::new();
        for relay in relays {
            if let Some(taken) = unique.iter().position(|old| old.url == relay.url) {
                unique.remove(taken);
            }
            unique.push(relay);
        }
        Ok(Self::from_relays(unique).with_remove(remove))
    }

    /// 全部的表项，档位在前。
    pub fn relays(&self) -> &[Relay] {
        &self.relays
    }

    /// 报给 App 的写法（配对码和 `/_wonton/api/tunnel`），档位在前。
    pub fn views(&self) -> Vec<RelayView> {
        self.relays.iter().map(Relay::view).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.relays.is_empty()
    }

    /// 档位的数量。
    pub fn tier_count(&self) -> usize {
        self.relays
            .last()
            .map_or(0, |relay| relay.tier as usize + 1)
    }

    /// 一档里的全部中继。没有这一档时是空的。
    pub fn tier(&self, tier: u8) -> &[Relay] {
        let start = self.relays.partition_point(|relay| relay.tier < tier);
        let end = self.relays.partition_point(|relay| relay.tier <= tier);
        &self.relays[start..end]
    }

    /// 把自己这一层盖在 `base` 上：先删 `remove` 里的地址和同 URL 的旧项，再追加自己的项。结果按
    /// 档位稳定排序，不是就地把项挪到新位置。
    pub fn overlay(&self, base: &RelayTable) -> RelayTable {
        let mut relays: Vec<Relay> = base
            .relays
            .iter()
            .filter(|relay| {
                !self.remove.contains(&relay.url)
                    && !self.relays.iter().any(|own| own.url == relay.url)
            })
            .cloned()
            .collect();
        relays.extend(self.relays.iter().cloned().map(canonical_relay));
        let mut unique: Vec<Relay> = Vec::new();
        for relay in relays {
            if let Some(taken) = unique.iter().position(|old| old.url == relay.url) {
                unique.remove(taken);
            }
            unique.push(relay);
        }
        let mut remove = base.remove.clone();
        for url in &self.remove {
            if !remove.contains(url) {
                remove.push(url.clone());
            }
        }
        RelayTable {
            relays: RelayTable::from_relays(unique).relays,
            remove,
        }
    }

    /// 删掉这些地址（连同它们自己的项）。和 [`overlay`](Self::overlay) 的删除标记一样，合并时还盖底表。
    pub fn without_urls(&self, urls: &[RelayUrl]) -> RelayTable {
        let relays = self
            .relays
            .iter()
            .filter(|relay| !urls.contains(&relay.url))
            .cloned()
            .collect();
        let mut remove = self.remove.clone();
        for url in urls {
            if !remove.contains(url) {
                remove.push(url.clone());
            }
        }
        RelayTable { relays, remove }
    }

    /// 需要 n0 兜底时把 n0 放在最后一档。已经显式写了 n0（或者显式删了某台）就不重复加。
    pub fn with_n0_last(&self, enabled: bool) -> RelayTable {
        if !enabled {
            return self.clone();
        }
        let n0: Vec<Relay> = n0_relays(0)
            .filter(|relay| {
                !self.remove.contains(&relay.url)
                    && !self.relays.iter().any(|own| own.url == relay.url)
            })
            .collect();
        if n0.is_empty() {
            return self.clone();
        }
        let tier = self
            .relays
            .last()
            .map_or(0, |relay| relay.tier.saturating_add(1));
        let mut relays = self.relays.clone();
        relays.extend(n0.into_iter().map(|relay| Relay { tier, ..relay }));
        RelayTable {
            relays: RelayTable::from_relays(relays).relays,
            remove: self.remove.clone(),
        }
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

/// 给界面、发现文件和日志用的规范化文字：根路径保留 `/`，非根路径去掉尾斜杠（`/relay/` → `/relay`）。
///
/// `Url` 序列化（以及 iroh 1.5 的 `RelayUrl`）会给非根路径补尾斜杠；配对码、连接状态和设置页
/// 统一成这里的一种写法才互相相等。
pub fn relay_url_text(url: &RelayUrl) -> String {
    url_text(&url.clone().into())
}

/// 同 [`relay_url_text`]，参数是已经去过 userinfo/查询/片段的 [`Url`]。
///
/// 只看路径部分的斜杠，`?`/`#` 前面也照样去掉（`/relay/?x=1` → `/relay?x=1`）；根路径的 `/` 保留。
pub fn url_text(url: &Url) -> String {
    let mut text = url.as_str().to_owned();
    let cut = text.find(['?', '#']).unwrap_or(text.len());
    if cut > 0 && text.as_bytes()[cut - 1] == b'/' && url.path() != "/" {
        text.remove(cut - 1);
    }
    text
}

/// 中继域名不带 FQDN 尾点，使配对码、中继表和连接状态使用同一个 key（#200）。
///
/// iroh 内置中继带尾点，webpki 允许它作为 TLS 参照名；用户报告在其网络中去掉尾点才能配对。
/// 在配置和连接信息入口统一写法，不改 TLS 校验或信任根。
pub fn canonical_url(url: RelayUrl) -> RelayUrl {
    let Some(domain) = url.domain() else {
        return url;
    };
    let host = domain.trim_end_matches('.');
    if host == domain || host.is_empty() {
        return url;
    }
    let host = host.to_owned();
    let mut inner = Url::from(url);
    inner
        .set_host(Some(&host))
        .expect("removing the DNS root dot preserves a valid host");
    RelayUrl::from(inner)
}

/// [`RelayView`] 的地址规范成一种写法（[`canonical_url`]）。
pub fn canonical_view(view: RelayView) -> RelayView {
    RelayView {
        url: canonical_url(view.url),
        ..view
    }
}

/// [`Relay`] 的地址规范成一种写法（[`canonical_url`]）。
pub fn canonical_relay(relay: Relay) -> Relay {
    Relay {
        url: canonical_url(relay.url),
        ..relay
    }
}

/// 这个地址是不是 n0 公共中继（[`n0_relays`] 那一组，含强制预发时的预发中继）。
///
/// 映射开着、路径走 n0 公共中继时带宽不够传画面，App 要如实提示（ADR-0072）；自建中继不提示。
/// iroh 报告的中继路径地址就是配置里的 URL，和这里用的是同一份表。
pub fn is_public_relay(url: &RelayUrl) -> bool {
    static N0: LazyLock<HashSet<RelayUrl>> =
        LazyLock::new(|| n0_relays(0).map(|relay| relay.url).collect());
    N0.contains(&canonical_url(url.clone()))
}

/// 报错时打的地址：userinfo 里的令牌换成 `<redacted>`，令牌不进日志。
fn redact_item(item: &str) -> String {
    let Some((scheme, rest)) = item.split_once("://") else {
        return item.to_owned();
    };
    // userinfo 只在 authority 里；query 里的 `@` 不算。
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let authority = match authority.split_once('@') {
        Some((_, host)) => format!("<redacted>@{host}"),
        None => authority.to_owned(),
    };
    format!("{scheme}://{authority}{tail}")
}

/// 解析一项中继地址：`https://TOKEN@relay.example.org#7843`。令牌取 userinfo 的用户名（按百分号编码
/// 解码），解析后从地址里去掉；`#` 后面是 QUIC 端口，空着用默认端口。
pub fn relay_from_item(item: &str, tier: u8) -> Result<Relay, String> {
    let mut url =
        Url::parse(item).map_err(|e| format!("{}：不是合法的地址：{e}", redact_item(item)))?;
    if url.scheme() != "https" {
        return Err(format!("{}：中继地址要用 https", redact_item(item)));
    }
    let quic_port = match url.fragment() {
        None => None,
        Some(fragment) => {
            let port = fragment
                .parse::<u16>()
                .map_err(|_| format!("{}：`#` 后面要写端口号，比如 #7843", redact_item(item)))?;
            if port == 0 {
                return Err(format!("{}：QUIC 端口不能是 0", redact_item(item)));
            }
            url.set_fragment(None);
            Some(port)
        }
    };
    // `https://@host`：写了 userinfo 却是空的，多半是令牌丢了，报出来而不是当没写。
    // 只看 authority 段；query 里的 `@` 不是 userinfo。
    let after_scheme = item.split_once("://").map_or(item, |(_, rest)| rest);
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    if authority.contains('@') && url.username().is_empty() && url.password().is_none() {
        return Err(format!("{}：令牌不能是空的", redact_item(item)));
    }
    let token = match (url.username(), url.password()) {
        ("", None) => None,
        (_, Some(_)) => {
            return Err(format!(
                "{}：令牌写在用户名里，不能有密码",
                redact_item(item)
            ));
        }
        (name, None) => {
            let decoded = percent_decode_str(name)
                .decode_utf8()
                .map_err(|_| format!("{}：令牌不是合法的 UTF-8", redact_item(item)))?;
            if decoded.is_empty() {
                return Err(format!("{}：令牌不能是空的", redact_item(item)));
            }
            Some(RelayToken::new(decoded.into_owned()))
        }
    };
    url.set_username("")
        .map_err(|_| format!("{}：去不掉 userinfo", redact_item(item)))?;
    let relay = Relay {
        url: canonical_url(RelayUrl::from(url)),
        token,
        quic_port,
        tier,
    };
    relay.validate()?;
    Ok(relay)
}

/// `n0` 关键字那一组：iroh 认的默认中继（设了 `IROH_FORCE_STAGING_RELAYS` 时是 n0 的预发环境，
/// 和 `presets::N0` 的选择一致）。
pub fn n0_relays(tier: u8) -> impl Iterator<Item = Relay> {
    default_relay_mode()
        .relay_map()
        .relays::<Vec<_>>()
        .into_iter()
        .map(move |config| Relay {
            // iroh 内置的主机名带 FQDN 尾点，规范成我们的一种写法（canonical_url 的说明）。
            url: canonical_url(config.url.clone()),
            token: None,
            // n0 自己开的 QUIC 端口就是 iroh 的默认端口，不单独写。
            quic_port: None,
            tier,
        })
}

/// 回退判定给出的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierAction {
    /// 把这一档加进 endpoint 的中继表。
    Enable(u8),
    /// 把这一档从 endpoint 的中继表撤掉。
    Disable(u8),
}

/// 分档回退的判定：启用中的最优先档全部连不上、持续超过阈值才往下一档走；低档的中继恢复可达就
/// 撤回来。纯逻辑，`now` 和两个信号都由调用方给。
#[derive(Debug)]
pub struct TierFallback {
    /// 已放进 endpoint 的档数（从最优先的档起连续的几档），至少 1。
    enabled: usize,
    /// 启用中的档全部连不上的起点；有任何中继连上就清掉。
    all_down_since: Option<Instant>,
}

impl TierFallback {
    pub fn new() -> Self {
        Self {
            enabled: 1,
            all_down_since: None,
        }
    }

    /// 表换了以后从头算：只留最优先一档，全断计时清零。
    pub fn reset(&mut self) {
        self.enabled = 1;
        self.all_down_since = None;
    }

    /// 已启用到第几档（`enabled` 档 0..n 都在 endpoint 的中继表里）。
    pub fn enabled_tiers(&self) -> usize {
        self.enabled
    }

    /// 根据两组信号算下一步，`after` 是阈值，测试用小值。
    ///
    /// - `connected`：连上的中继。iroh 的 [`Endpoint::home_relay_status`] 只报当前家中继一个，而家中继
    ///   总是它探测里最优的已连中继（插拔中继会触发重新探测），所以「没有任何已连中继」等价于「启用
    ///   中的档全部连不上」。
    /// - `reachable`：低档里 TCP 端口连得上的中继。家里继被 iroh 换到高档后，低档恢复了它也不会马上
    ///   换回去（近 5 分钟的探测里要快得多才换），家里继状态看不到低档的恢复，只能自己探。
    pub fn step(
        &mut self,
        table: &RelayTable,
        connected: &[RelayUrl],
        reachable: &[RelayUrl],
        now: Instant,
        after: Duration,
    ) -> Option<TierAction> {
        // 撤档：低档有中继的端口连得上了（进程活着），最靠后的那档就不需要了。宁可乐观：撤错了，
        // 全断的计时会在阈值后把档再加回来，代价是 30 秒的慢路径。
        if (0..self.enabled - 1).any(|tier| {
            table
                .tier(tier as u8)
                .iter()
                .any(|relay| reachable.contains(&relay.url))
        }) {
            self.enabled -= 1;
            return Some(TierAction::Disable(self.enabled as u8));
        }
        let any_up = (0..self.enabled).any(|tier| {
            table
                .tier(tier as u8)
                .iter()
                .any(|relay| connected.contains(&relay.url))
        });
        if any_up {
            self.all_down_since = None;
            return None;
        }
        let since = self.all_down_since.get_or_insert(now);
        if self.enabled < table.tier_count() && now.saturating_duration_since(*since) >= after {
            let tier = self.enabled as u8;
            self.enabled += 1;
            // 新的启用范围重新计时：刚加进来的档还没连上，不能马上又往下一档走。
            self.all_down_since = None;
            return Some(TierAction::Enable(tier));
        }
        None
    }
}

impl Default for TierFallback {
    fn default() -> Self {
        Self::new()
    }
}

/// 分档回退任务的手柄：看当前启用到第几档，测试和排障用。任务自己看着 endpoint 关掉结束，手柄
/// 丢了它也照跑。
#[derive(Debug)]
pub struct TierFallbackHandle {
    enabled: tokio::sync::watch::Receiver<u8>,
    /// 换表用：回退任务每拍读最新一份，换表也算一次重置。
    tables: tokio::sync::watch::Sender<RelayTable>,
    /// 回退任务。丢掉句柄不 abort：任务自己盯到 endpoint 关闭才结束，误丢不会悄悄关掉回退。
    _task: tokio::task::JoinHandle<()>,
}

impl TierFallbackHandle {
    pub fn enabled_tiers(&self) -> u8 {
        *self.enabled.borrow()
    }

    /// 换一张表：下一拍用新表，并从最优先一档重新算。
    pub fn set_table(&self, table: RelayTable) {
        self.tables.send_replace(table);
    }

    /// 等启用档数变成 `tiers`。
    pub async fn wait_enabled_tiers(&mut self, tiers: u8) {
        self.enabled
            .wait_for(|enabled| *enabled == tiers)
            .await
            .expect("回退任务没在了");
    }
}

/// 恢复探针：每台被撤档的中继一个自己的 endpoint。
///
/// 为什么不让一个探针 endpoint 装多台中继：`home_relay_status` 只报当前家中继一个，多台装在
/// 一个 map 里时，剩下的谁插过、谁连上没有第二个信号，只能靠“缺的就插”每拍对一遍——而每次
/// `insert_relay` 都是一次 `RelayMapChange`，iroh 会为它跑一遍完整 net_report（每台中继 3 个 HTTPS
/// 探测），拒绝期间变成每秒几个新连接的放大回路。单台中继的 map 没有这个问题：它的家中继就是那台，
/// `home_relay_status` 直接是那台的握手结论，档位不变时一次都不用动它。
///
/// 探针仍然长寿命：档位变了只增删差集，已有的不重建（不重新 bind、不重新握手）。恢复判定靠 iroh
/// 自己的 10 毫秒起、封顶 16 秒的退避重连，加上它每 20–26 秒的周期 net_report，我们不再另加节奏。
struct Probes {
    endpoints: HashMap<RelayUrl, Endpoint>,
}

impl Probes {
    fn new() -> Self {
        Self {
            endpoints: HashMap::new(),
        }
    }

    /// 关掉不再需要探的 endpoint。
    fn retain(&mut self, wanted: &[Relay]) {
        let stale: Vec<RelayUrl> = self
            .endpoints
            .keys()
            .filter(|url| !wanted.iter().any(|relay| &relay.url == *url))
            .cloned()
            .collect();
        for url in stale {
            if let Some(endpoint) = self.endpoints.remove(&url) {
                tokio::spawn(async move { endpoint.close().await });
            }
        }
    }

    async fn close(self) {
        for endpoint in self.endpoints.into_values() {
            endpoint.close().await;
        }
    }

    /// 真正连上的（带令牌握手通过）中继。
    fn connected(&self) -> Vec<RelayUrl> {
        self.endpoints
            .iter()
            .filter(|(url, endpoint)| {
                endpoint
                    .home_relay_status()
                    .get()
                    .iter()
                    .any(|status| status.is_connected() && status.url() == *url)
            })
            .map(|(url, _)| url.clone())
            .collect()
    }
}

/// 起一个只连 `map` 里这些中继的探针 endpoint：最小 preset、随机身份、不做直连和地址发现。它的
/// 家中继状态就是「带令牌的真握手」的结论：中继进程活着却不收我们（令牌被吊销、配额用完、反向
/// 代理后面后端挂了）时，握手到不了 Connected，不会误判成恢复（只看 TCP 端口就会来回切）。
///
/// 准入按 endpoint ID 的话（ADR-0048 里收费后的 http 回调），探针的临时身份不在名单里：探针会一直
/// 判成不可达，停在低档上。这是保守方向——连接不断，只是恢复判断变慢，等有按身份的探针办法再换。
async fn spawn_probe(map: RelayMap, tls: Option<CaTlsConfig>) -> anyhow::Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(map));
    if let Some(tls) = tls {
        builder = builder.ca_tls_config(tls);
    }
    crate::off_workers(move || builder.bind())
        .await
        .context("绑定中继探针 endpoint")?
        .context("绑定中继探针 endpoint")
}

fn log_relay_url(url: &RelayUrl) -> &str {
    url.as_str().split(['?', '#']).next().unwrap_or_default()
}

/// 按当前被撤的档（`wanted` 档 0..n）对齐探针覆盖集合，返回真正连上的中继——恢复信号。没有激活
/// 回退（wanted == 0）时关掉全部探针。
async fn probe_signal(
    probe: &mut Probes,
    table: &RelayTable,
    wanted: usize,
    tls: Option<CaTlsConfig>,
) -> Vec<RelayUrl> {
    let wanted: Vec<Relay> = (0..wanted)
        .flat_map(|tier| table.tier(tier as u8))
        .cloned()
        .collect();
    probe.retain(&wanted);
    for relay in &wanted {
        if probe.endpoints.contains_key(&relay.url) {
            continue;
        }
        let map: RelayMap = std::iter::once(Arc::new(relay.relay_config())).collect();
        match spawn_probe(map, tls.clone()).await {
            Ok(endpoint) => {
                probe.endpoints.insert(relay.url.clone(), endpoint);
            }
            Err(error) => {
                tracing::warn!(%error, url = %log_relay_url(&relay.url), "中继表：恢复探针起不来，这拍不探这台");
            }
        }
    }
    probe.connected()
}

/// 盯着 iroh 的家中继状态跑分档回退：到点了就 [`Endpoint::insert_relay`] 下一档、
/// [`Endpoint::remove_relay`] 撤掉不要的档。endpoint 关掉后自己结束。
///
/// `probe_tls` 是探针 endpoint 的 TLS 配置：生产里不设（用 iroh 默认的证书校验，和主 endpoint 一致），
/// 测试里传跳过校验（本地中继是自签证书）。探针的 TLS 立场要和主 endpoint 一样，否则会误判。
pub fn spawn_tier_fallback(
    endpoint: Endpoint,
    table: RelayTable,
    after: Duration,
    probe_tls: Option<CaTlsConfig>,
) -> TierFallbackHandle {
    let (enabled, watching) = tokio::sync::watch::channel(1);
    let (tables, mut table_rx) = tokio::sync::watch::channel(table);
    let task = tokio::spawn(async move {
        let mut fallback = TierFallback::new();
        let mut table = table_rx.borrow().clone();
        // 长寿命探针：只在回退激活期间存在，档位变了只增删差集。单台中继一个 endpoint 是为了让
        // 家中继状态直接等于那台的结论（见 `Probes`）。
        let mut probe = Probes::new();
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if endpoint.is_closed() {
                probe.close().await;
                return;
            }
            // 主人刚换了表：从头算，探针集合跟着新表对齐（probe_signal 会 retain）。
            if table_rx.has_changed().unwrap_or(false) {
                table = table_rx.borrow_and_update().clone();
                fallback.reset();
                enabled.send_replace(1);
            }
            let connected: Vec<RelayUrl> = endpoint
                .home_relay_status()
                .get()
                .iter()
                .filter(|status| status.is_connected())
                .map(|status| status.url().clone())
                .collect();
            let reachable_relays = probe_signal(
                &mut probe,
                &table,
                fallback.enabled_tiers().saturating_sub(1),
                probe_tls.clone(),
            )
            .await;
            match fallback.step(&table, &connected, &reachable_relays, Instant::now(), after) {
                Some(TierAction::Enable(tier)) => {
                    tracing::info!(tier, relays = ?table.tier(tier).iter().map(|r| log_relay_url(&r.url)).collect::<Vec<_>>(), "中继表：这一档全断超阈值，加进下一档");
                    for relay in table.tier(tier) {
                        // 上游 n0-computer/iroh#4642：insert_relay 换掉已有的项不会重连，令牌不换。
                        // 这里只加新档，地址不会重复；要换令牌得整个重建 endpoint。
                        endpoint
                            .insert_relay(relay.url.clone(), Arc::new(relay.relay_config()))
                            .await;
                    }
                    // 全断的高档同时撤出 map。iroh 会记住上次选中的路径，死档留在 map 里时它的传输
                    // 层可能迟迟没察觉服务端消失（实测 ping 都在正常收发），重拨就会一直往死路径发
                    // 握手，拨号超时也学不到别的路径。撤出 map 后重拨的候选只剩活着的档。
                    for lower in 0..tier {
                        for relay in table.tier(lower) {
                            endpoint.remove_relay(&relay.url).await;
                        }
                    }
                }
                Some(TierAction::Disable(tier)) => {
                    tracing::info!(tier, "中继表：低档有中继恢复可达，撤掉这一档");
                    for relay in table.tier(tier) {
                        endpoint.remove_relay(&relay.url).await;
                    }
                    // Enable 时撤掉的高档加回来：它们的端口刚被探测到可达。
                    for lower in 0..tier {
                        for relay in table.tier(lower) {
                            endpoint
                                .insert_relay(relay.url.clone(), Arc::new(relay.relay_config()))
                                .await;
                        }
                    }
                }
                None => {}
            }
            enabled.send_replace(fallback.enabled_tiers() as u8);
        }
    });
    TierFallbackHandle {
        enabled: watching,
        tables,
        _task: task,
    }
}

#[cfg(test)]
mod tests;
