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

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use iroh::endpoint::default_relay_mode;
use iroh::endpoint::presets;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey, Watcher};
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

/// 一台中继：地址、可选的准入令牌、档位（数字小的优先）。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relay {
    pub url: RelayUrl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<RelayToken>,
    pub tier: u8,
}

impl Relay {
    /// 从连接信息或配对码读回来的表项要过这一关：地址里不许藏令牌（令牌单独放），令牌不能是空的，
    /// 而且得是 https，令牌不走明文。
    pub fn validate(&self) -> Result<(), String> {
        if !self.url.username().is_empty() || self.url.password().is_some() {
            return Err(format!("{}：地址里不能带 userinfo，令牌要单独放", self.url));
        }
        if self.url.scheme() != "https" {
            return Err(format!("{}：中继地址要用 https", self.url));
        }
        if let Some(token) = &self.token
            && token.expose().is_empty()
        {
            return Err(format!("{}：令牌不能是空的", self.url));
        }
        Ok(())
    }

    /// iroh 认识的中继配置。自建中继按 ADR-0048 的部署约定开 QUIC 地址发现（UDP 7842），和 iroh
    /// 对 n0 中继的默认一样。
    fn relay_config(&self) -> RelayConfig {
        let mut config = RelayConfig::from(self.url.clone());
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
            .field("tier", &self.tier)
            .finish()
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayTable {
    relays: Vec<Relay>,
}

impl RelayTable {
    /// 不设 [`RELAYS_VAR`] 时的表：n0 的公共中继一档。
    pub fn builtin_n0() -> Self {
        Self {
            relays: n0_relays(0).collect(),
        }
    }

    /// 从连接信息或配对码里带来的表项组表。表项要先用 [`Relay::validate`] 校验过。
    pub fn from_relays(relays: Vec<Relay>) -> Self {
        let mut relays = relays;
        relays.sort_by_key(|relay| relay.tier);
        Self { relays }
    }

    /// 解析 [`RELAYS_VAR`] 的值。写得不合法时说清是哪一项、为什么，不静默跳过。
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut relays: Vec<Relay> = Vec::new();
        for (index, tier_text) in text.split(';').enumerate() {
            let tier = u8::try_from(index).map_err(|_| "档位太多".to_owned())?;
            for item in tier_text.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    return Err(format!("第 {} 档里有空的中继项", index + 1));
                }
                if item == N0 {
                    relays.extend(n0_relays(tier));
                } else {
                    relays.push(relay_from_url(item, tier)?);
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
        Ok(Self::from_relays(unique))
    }

    /// 全部的表项，档位在前。
    pub fn relays(&self) -> &[Relay] {
        &self.relays
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
}

/// 从中继表项建 iroh 的 [`RelayMap`]：只放前 `tiers` 档。
pub fn relay_map(relays: &[Relay], tiers: usize) -> RelayMap {
    relays
        .iter()
        .filter(|relay| (relay.tier as usize) < tiers)
        .map(|relay| Arc::new(relay.relay_config()))
        .collect()
}

/// 解析一项中继地址：`https://TOKEN@relay.example.org`。令牌取 userinfo 的用户名（按百分号编码
/// 解码），解析后从地址里去掉。
fn relay_from_url(item: &str, tier: u8) -> Result<Relay, String> {
    let mut url = Url::parse(item).map_err(|e| format!("{item}：不是合法的地址：{e}"))?;
    if url.scheme() != "https" {
        return Err(format!("{item}：中继地址要用 https"));
    }
    // `https://@host`：写了 userinfo 却是空的，多半是令牌丢了，报出来而不是当没写。
    let authority = item.split_once("://").map_or(item, |(_, rest)| rest);
    if authority.contains('@') && url.username().is_empty() && url.password().is_none() {
        return Err(format!("{item}：令牌不能是空的"));
    }
    let token = match (url.username(), url.password()) {
        ("", None) => None,
        (_, Some(_)) => return Err(format!("{item}：令牌写在用户名里，不能有密码")),
        (name, None) => {
            let decoded = percent_decode_str(name)
                .decode_utf8()
                .map_err(|_| format!("{item}：令牌不是合法的 UTF-8"))?;
            if decoded.is_empty() {
                return Err(format!("{item}：令牌不能是空的"));
            }
            Some(RelayToken::new(decoded.into_owned()))
        }
    };
    url.set_username("")
        .map_err(|_| format!("{item}：去不掉 userinfo"))?;
    Ok(Relay {
        url: RelayUrl::from(url),
        token,
        tier,
    })
}

/// `n0` 关键字那一组：iroh 认的默认中继（设了 `IROH_FORCE_STAGING_RELAYS` 时是 n0 的预发环境，
/// 和 `presets::N0` 的选择一致）。
fn n0_relays(tier: u8) -> impl Iterator<Item = Relay> {
    default_relay_mode()
        .relay_map()
        .relays::<Vec<_>>()
        .into_iter()
        .map(move |config| Relay {
            url: config.url.clone(),
            token: None,
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
#[derive(Debug, Clone)]
pub struct TierFallbackHandle {
    enabled: tokio::sync::watch::Receiver<u8>,
}

impl TierFallbackHandle {
    pub fn enabled_tiers(&self) -> u8 {
        *self.enabled.borrow()
    }

    /// 等启用档数变成 `tiers`。
    pub async fn wait_enabled_tiers(&mut self, tiers: u8) {
        self.enabled
            .wait_for(|enabled| *enabled == tiers)
            .await
            .expect("回退任务没在了");
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
    builder.bind().await.context("绑定中继探针 endpoint")
}

/// 按当前被撤的档（tiers 档以上全部启用时就是 0..wanted）对齐探针的覆盖集合，返回探针真正连上的
/// 中继——恢复信号。没有激活回退（wanted == 0）时关掉探针。
async fn probe_signal(
    probe: &mut Option<Endpoint>,
    table: &RelayTable,
    wanted: usize,
    tls: Option<CaTlsConfig>,
) -> Vec<RelayUrl> {
    if wanted == 0 {
        if let Some(p) = probe.take() {
            tokio::spawn(async move { p.close().await });
        }
        return Vec::new();
    }
    let wanted: Vec<Relay> = (0..wanted)
        .flat_map(|tier| table.tier(tier as u8))
        .cloned()
        .collect();
    if probe.as_ref().is_none_or(Endpoint::is_closed) {
        if let Some(dead) = probe.take() {
            drop(dead);
        }
        match spawn_probe(relay_map(table.relays(), 0), tls).await {
            Ok(p) => *probe = Some(p),
            Err(error) => {
                tracing::warn!(%error, "中继表：恢复探针起不来，这拍不判恢复");
                return Vec::new();
            }
        }
    }
    let Some(p) = probe.as_ref() else {
        return Vec::new();
    };
    // 对齐覆盖集合：多的撤掉、缺的加进。
    let held: Vec<RelayUrl> = p
        .home_relay_status()
        .get()
        .iter()
        .map(|status| status.url().clone())
        .collect();
    for url in &held {
        if !wanted.iter().any(|relay| &relay.url == url) {
            p.remove_relay(url).await;
        }
    }
    for relay in &wanted {
        if !held.contains(&relay.url) {
            p.insert_relay(relay.url.clone(), Arc::new(relay.relay_config()))
                .await;
        }
    }
    p.home_relay_status()
        .get()
        .iter()
        .filter(|status| status.is_connected())
        .map(|status| status.url().clone())
        .collect()
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
    tokio::spawn(async move {
        let mut fallback = TierFallback::new();
        // 长寿命探针：只在回退激活期间存在，档数变了不改建，直接对它 insert/remove_relay 对齐被撤
        // 的档集合（和主 endpoint 同一套调用）。不重建就不必每次重新 bind + 重新握手：换档的瞬间
        // 主 endpoint 也在重连同一批中继，两边不会互相抢拨号。
        let mut probe: Option<Endpoint> = None;
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if endpoint.is_closed() {
                return;
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
                    tracing::info!(tier, relays = ?table.tier(tier).iter().map(|r| r.url.clone()).collect::<Vec<_>>(), "中继表：这一档全断超阈值，加进下一档");
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
    TierFallbackHandle { enabled: watching }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> RelayUrl {
        text.parse().unwrap()
    }

    fn relay(text: &str, tier: u8) -> Relay {
        Relay {
            url: url(text),
            token: None,
            tier,
        }
    }

    #[test]
    fn the_default_table_is_the_n0_group() {
        let table = RelayTable::builtin_n0();
        assert_eq!(RelayTable::parse("n0").unwrap(), table);
        assert_eq!(table.tier_count(), 1);
        assert!(!table.tier(0).is_empty());
        assert!(table.tier(0).iter().all(|relay| relay.tier == 0));
        assert!(table.tier(0).iter().all(|relay| relay.token.is_none()));
    }

    #[test]
    fn a_token_in_the_userinfo_is_taken_out_of_the_url() {
        let table = RelayTable::parse("https://tok@a.example").unwrap();
        assert_eq!(table.tier(0).len(), 1);
        assert_eq!(table.tier(0)[0].url, url("https://a.example/"));
        assert_eq!(table.tier(0)[0].token.as_ref().unwrap().expose(), "tok");
        // 百分号编码的令牌按原文解码。
        let table = RelayTable::parse("https://a%40b%3Ac@example.org;n0").unwrap();
        assert_eq!(table.tier(0)[0].token.as_ref().unwrap().expose(), "a@b:c");
        assert_eq!(table.tier(1).len(), RelayTable::builtin_n0().tier(0).len());
    }

    #[test]
    fn tiers_keep_their_order_and_n0_can_sit_in_any_tier() {
        let table = RelayTable::parse("https://b.example,https://tok@a.example;n0").unwrap();
        assert_eq!(table.tier_count(), 2);
        assert_eq!(table.tier(0)[0].url, url("https://b.example/"));
        assert_eq!(table.tier(0)[1].url, url("https://a.example/"));
        assert_eq!(table.tier(0)[1].token.as_ref().unwrap().expose(), "tok");
        assert_eq!(table.tier(1).len(), RelayTable::builtin_n0().tier(0).len());
    }

    #[test]
    fn a_later_entry_for_the_same_address_replaces_the_earlier_one() {
        let table = RelayTable::parse("https://a.example;n0").unwrap();
        assert_eq!(table.tier_count(), 2);
        // 同一地址再写一次，档位以最后一次为准。
        let table = RelayTable::parse("https://a.example,https://tok@a.example").unwrap();
        assert_eq!(table.tier_count(), 1);
        assert_eq!(table.tier(0).len(), 1);
        assert_eq!(table.tier(0)[0].token.as_ref().unwrap().expose(), "tok");
    }

    #[test]
    fn invalid_values_name_the_offending_item() {
        for (text, expected) in [
            ("", "空的中继项"),
            ("  ", "空的中继项"),
            ("n0;", "空的中继项"),
            ("n0,,n0", "空的中继项"),
            ("notaurl", "不是合法的地址"),
            ("http://a.example", "要用 https"),
            ("https://tok:pw@a.example", "不能有密码"),
            ("https://@a.example", "令牌不能是空"),
            ("https://%ff@a.example", "UTF-8"),
        ] {
            let error = RelayTable::parse(text).unwrap_err();
            assert!(error.contains(expected), "{text}：{error}");
        }
        assert!(
            RelayTable::parse(vec!["n0"; 300].join(";").as_str())
                .unwrap_err()
                .contains("档位太多")
        );
    }

    #[test]
    fn wire_entries_must_not_carry_the_token_in_the_url() {
        let mut entry = relay("https://a.example", 0);
        assert_eq!(entry.validate(), Ok(()));
        entry.url = "https://tok@a.example".parse().unwrap();
        assert!(entry.validate().unwrap_err().contains("userinfo"));
        entry.url = "http://a.example".parse().unwrap();
        assert!(entry.validate().unwrap_err().contains("https"));
        entry = relay("https://a.example", 0);
        entry.token = Some(RelayToken::new(""));
        assert!(entry.validate().unwrap_err().contains("空"));
    }

    #[test]
    fn the_map_only_holds_the_requested_tiers() {
        let table = RelayTable::parse("https://a.example;n0").unwrap();
        let map = relay_map(table.relays(), 1);
        assert_eq!(map.len(), 1);
        assert!(map.contains(&url("https://a.example/")));
        let map = relay_map(table.relays(), 2);
        assert_eq!(map.len(), 1 + RelayTable::builtin_n0().tier(0).len());
        // 令牌进了 iroh 的配置，不在地址里。
        let config = map.get(&url("https://a.example/")).unwrap();
        assert_eq!(config.auth_token.as_deref(), None);
        let table = RelayTable::parse("https://tok@a.example").unwrap();
        let map = relay_map(table.relays(), 1);
        let config = map.get(&url("https://a.example/")).unwrap();
        assert_eq!(config.auth_token.as_deref(), Some("tok"));
    }

    #[test]
    fn fallback_waits_out_the_threshold_before_enabling_the_next_tier() {
        let table =
            RelayTable::parse("https://a.example;https://b.example;https://c.example").unwrap();
        let mut fallback = TierFallback::new();
        let start = Instant::now();
        let step = |fallback: &mut TierFallback, now, connected: Vec<&str>| {
            fallback.step(
                &table,
                &connected.iter().map(|text| url(text)).collect::<Vec<_>>(),
                &[],
                now,
                Duration::from_secs(30),
            )
        };
        // 顶档连着：什么都不做。
        assert_eq!(step(&mut fallback, start, vec!["https://a.example/"]), None);
        // 全断，但还没到阈值：等。
        assert_eq!(step(&mut fallback, start, vec![]), None);
        assert_eq!(
            step(&mut fallback, start + Duration::from_secs(29), vec![]),
            None
        );
        // 到了阈值：把下一档加进来，并重新计时。
        assert_eq!(
            step(&mut fallback, start + Duration::from_secs(30), vec![]),
            Some(TierAction::Enable(1))
        );
        // 刚加进来的档还没连上，不马上走下一档。
        assert_eq!(
            step(&mut fallback, start + Duration::from_secs(35), vec![]),
            None
        );
        assert_eq!(
            step(&mut fallback, start + Duration::from_secs(65), vec![]),
            Some(TierAction::Enable(2))
        );
        // 全部档都启用后不再加。
        assert_eq!(
            step(&mut fallback, start + Duration::from_secs(100), vec![]),
            None
        );
    }

    #[test]
    fn fallback_removes_a_tier_once_a_lower_one_is_reachable_again() {
        let table =
            RelayTable::parse("https://a.example;https://b.example;https://c.example").unwrap();
        let mut fallback = TierFallback::new();
        let start = Instant::now();
        let step = |fallback: &mut TierFallback, now, reachable: Vec<&str>| {
            fallback.step(
                &table,
                &[],
                &reachable.iter().map(|text| url(text)).collect::<Vec<_>>(),
                now,
                Duration::from_secs(30),
            )
        };
        // 预热计时，然后逐档启用。
        assert_eq!(step(&mut fallback, start, vec![]), None);
        let t1 = start + Duration::from_secs(30);
        assert_eq!(step(&mut fallback, t1, vec![]), Some(TierAction::Enable(1)));
        assert_eq!(
            step(&mut fallback, t1 + Duration::from_secs(1), vec![]),
            None
        );
        let t2 = t1 + Duration::from_secs(31);
        assert_eq!(step(&mut fallback, t2, vec![]), Some(TierAction::Enable(2)));
        // 低档有端口连得上的中继（顶档恢复了）：最靠后的那档先撤。
        assert_eq!(
            step(&mut fallback, t2, vec!["https://a.example/"]),
            Some(TierAction::Disable(2))
        );
        // 还连得上，继续撤。
        assert_eq!(
            step(&mut fallback, t2, vec!["https://a.example/"]),
            Some(TierAction::Disable(1))
        );
        // 只剩顶档，不再撤。
        assert_eq!(step(&mut fallback, t2, vec!["https://a.example/"]), None);
        // 低档全断时撤档不会发生。
        assert_eq!(step(&mut fallback, t2, vec![]), None);
        // 中间档连得上也算：它一恢复，它后面的档就不需要了。
        let mut fallback = TierFallback::new();
        assert_eq!(step(&mut fallback, t2, vec![]), None);
        let t4 = t2 + Duration::from_secs(30);
        assert_eq!(step(&mut fallback, t4, vec![]), Some(TierAction::Enable(1)));
        assert_eq!(
            step(&mut fallback, t4 + Duration::from_secs(1), vec![]),
            None
        );
        let t5 = t4 + Duration::from_secs(31);
        assert_eq!(step(&mut fallback, t5, vec![]), Some(TierAction::Enable(2)));
        assert_eq!(
            step(&mut fallback, t5, vec!["https://b.example/"]),
            Some(TierAction::Disable(2))
        );
        // 顶档还断着：第二档留着顶班。
        assert_eq!(step(&mut fallback, t5, vec![]), None);
    }

    #[test]
    fn a_recovered_top_tier_resets_the_timer() {
        let table = RelayTable::parse("https://a.example;https://b.example").unwrap();
        let mut fallback = TierFallback::new();
        let start = Instant::now();
        let step = |fallback: &mut TierFallback, now, connected: Vec<&str>| {
            fallback.step(
                &table,
                &connected.iter().map(|text| url(text)).collect::<Vec<_>>(),
                &[],
                now,
                Duration::from_secs(30),
            )
        };
        assert_eq!(step(&mut fallback, start, vec![]), None);
        // 断了 20 秒时恢复一下，计时清零；再断要重新等满 30 秒。
        assert_eq!(
            step(
                &mut fallback,
                start + Duration::from_secs(20),
                vec!["https://a.example/"]
            ),
            None
        );
        assert_eq!(
            step(&mut fallback, start + Duration::from_secs(25), vec![]),
            None
        );
        assert_eq!(
            step(&mut fallback, start + Duration::from_secs(55), vec![]),
            Some(TierAction::Enable(1))
        );
    }
}
