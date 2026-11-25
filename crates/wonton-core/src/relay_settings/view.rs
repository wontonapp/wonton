//! 把 iroh 的家中继状态翻成协议里的 [`RelayState`]：只有当前家中继有确切状态，其余是未知。

use std::collections::HashMap;

use iroh::{Endpoint, RelayUrl, Watcher};
use wonton_protocol::RelayState;

/// 中继拒绝的原因可能很长（整段 HTML），设置页一行放不下。
const MAX_REASON: usize = 200;

/// 当前 endpoint 上每台中继的状态。
pub(super) fn statuses(endpoint: &Endpoint) -> HashMap<RelayUrl, RelayState> {
    let mut states = HashMap::new();
    let mut watcher = endpoint.home_relay_status();
    for status in watcher.get() {
        let state = if let Some(reason) = status.auth_denied_reason() {
            RelayState::Refused {
                reason: Some(truncate(reason)),
            }
        } else if status.is_connected() {
            RelayState::Connected
        } else if let Some(error) = status.last_error() {
            RelayState::Unreachable {
                reason: Some(truncate(&error.to_string())),
            }
        } else {
            RelayState::Unknown
        };
        states.insert(status.url().clone(), state);
    }
    states
}

pub(super) fn state_of(states: &HashMap<RelayUrl, RelayState>, url: &RelayUrl) -> RelayState {
    states.get(url).cloned().unwrap_or(RelayState::Unknown)
}

fn truncate(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(MAX_REASON).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_reason_is_cut_at_a_character_boundary() {
        let text = "错".repeat(MAX_REASON + 10);
        let cut = truncate(&text);
        assert_eq!(cut.chars().count(), MAX_REASON + 1);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn an_unknown_address_is_unknown() {
        let url: RelayUrl = "https://relay.example.org".parse().unwrap();
        assert_eq!(state_of(&HashMap::new(), &url), RelayState::Unknown);
    }
}
