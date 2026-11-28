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

mod fallback;
mod n0;
mod parse;
mod relay;
mod table;
mod url;

#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use self::parse::redact_item;
#[cfg(test)]
use iroh::RelayUrl;

pub use fallback::{
    FALLBACK_AFTER, TierAction, TierFallback, TierFallbackHandle, spawn_tier_fallback,
};
pub use n0::{is_public_relay, n0_relays};
pub use parse::relay_from_item;
pub use relay::{Relay, RelayToken, RelayView, relay_map};
pub use table::{RELAYS_VAR, RelayTable};
pub use url::{canonical_relay, canonical_url, canonical_view, relay_url_text, url_text};

#[cfg(test)]
mod tests;
