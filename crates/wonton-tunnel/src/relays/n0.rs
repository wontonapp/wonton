use std::collections::HashSet;
use std::sync::LazyLock;

use iroh::RelayUrl;
use iroh::endpoint::default_relay_mode;

use super::relay::Relay;
use super::url::canonical_url;

/// 这个地址是不是 n0 公共中继（[`n0_relays`] 那一组，含强制预发时的预发中继）。
///
/// 映射开着、路径走 n0 公共中继时带宽不够传画面，App 要如实提示（ADR-0072）；自建中继不提示。
/// iroh 报告的中继路径地址就是配置里的 URL，和这里用的是同一份表。
pub fn is_public_relay(url: &RelayUrl) -> bool {
    static N0: LazyLock<HashSet<RelayUrl>> =
        LazyLock::new(|| n0_relays(0).map(|relay| relay.url).collect());
    N0.contains(&canonical_url(url.clone()))
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
