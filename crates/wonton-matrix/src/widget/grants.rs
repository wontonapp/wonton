//! 批给 widget 的能力（ADR-0096）：页面请求的 ∩ 清单声明的 ∩ 核心允许的。
//!
//! 按 MSC2762 的能力串算：SDK 把请求解析成 [`Capabilities`]，这里序列化回字符串逐条比对，再解析回去交给驱动。
//! SDK 不认识的能力（粘性事件、`m.always_on_screen` 等）解析时已经丢了，页面拿不到。
//!
//! 发状态事件时 `state_key` 只能是自己的：等于自己的用户 ID，或以 `<自己>_`、`_<自己>_` 开头（MSC3779，
//! Element Call 的 `m.call.member` 就这样写）。SDK 的过滤只认确切的 `state_key`，表达不了前缀，所以页面
//! 要“任意 state_key”时只批 `#<自己>` 这一条；带具体 `state_key` 的按前缀规则核。

use std::future::Future;

use matrix_sdk::SendOutsideWasm;
use matrix_sdk::ruma::{OwnedUserId, UserId};
use matrix_sdk::widget::{Capabilities, CapabilitiesProvider};

/// 带事件类型的能力前缀。`timeline:`（别的房间）不在里面：页面只拿到打开它的那个房间。
const TYPED: [&str; 6] = [
    "org.matrix.msc2762.send.event",
    "org.matrix.msc2762.receive.event",
    SEND_STATE,
    "org.matrix.msc2762.receive.state_event",
    "org.matrix.msc3819.send.to_device",
    "org.matrix.msc3819.receive.to_device",
];

const SEND_STATE: &str = "org.matrix.msc2762.send.state_event";

/// 不带事件类型、SDK 认识的能力。
const BARE: [&str; 5] = [
    "io.element.requires_client",
    "org.matrix.msc4039.download_file",
    "org.matrix.msc4157.send.delayed_event",
    "org.matrix.msc4157.update_delayed_event",
    "org.matrix.msc4515.rtc_transports",
];

pub(crate) struct Grants {
    declared: Vec<String>,
    own: OwnedUserId,
}

impl Grants {
    pub(crate) fn new(declared: Vec<String>, own: OwnedUserId) -> Self {
        Self { declared, own }
    }
}

impl CapabilitiesProvider for Grants {
    fn acquire_capabilities(
        &self,
        requested: Capabilities,
    ) -> impl Future<Output = Capabilities> + SendOutsideWasm {
        let granted = grant(&requested, &self.declared, &self.own);
        std::future::ready(granted)
    }
}

/// 批下来的能力。请求了却没批的记一条日志：页面按规范只拿到实际批准的。
pub(crate) fn grant(requested: &Capabilities, declared: &[String], own: &UserId) -> Capabilities {
    let wanted = strings(requested);
    let declared = &with_sdk_spelling(declared);
    let mut granted: Vec<String> = Vec::new();
    for capability in wanted.iter().flat_map(|c| narrow(c, declared, own)) {
        // 不限的请求收窄后可能和页面另外点名的那条重复。
        if !granted.contains(&capability) {
            granted.push(capability);
        }
    }
    let refused: Vec<&String> = wanted
        .iter()
        .filter(|capability| !granted.contains(capability))
        .collect();
    if !refused.is_empty() {
        tracing::info!(?refused, ?granted, "widget asked for more than it gets");
    }
    serde_json::from_value(serde_json::Value::from(granted)).unwrap_or_default()
}

/// 声明再按 SDK 的解析过一遍：SDK 把请求里的事件类型换成它认的写法（`m.call.member` 解析成
/// `org.matrix.msc3401.call.member`），声明不跟着换就对不上。原样的也留着。
fn with_sdk_spelling(declared: &[String]) -> Vec<String> {
    let parsed: Capabilities =
        serde_json::from_value(serde_json::Value::from(declared.to_vec())).unwrap_or_default();
    let mut all = declared.to_vec();
    for capability in strings(&parsed) {
        if !all.contains(&capability) {
            all.push(capability);
        }
    }
    all
}

fn strings(capabilities: &Capabilities) -> Vec<String> {
    serde_json::to_value(capabilities)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// 一条请求能批成哪几条：原样、收窄到声明里具体的那几条，或者一条都不批。
fn narrow(capability: &str, declared: &[String], own: &UserId) -> Vec<String> {
    if BARE.contains(&capability) {
        return if declared.iter().any(|d| d == capability) {
            vec![capability.to_owned()]
        } else {
            Vec::new()
        };
    }
    let Some((prefix, filter)) = capability.split_once(':') else {
        return Vec::new();
    };
    if !TYPED.contains(&prefix) {
        return Vec::new();
    }
    let (event_type, key) = split_filter(filter);
    // 清单里同一前缀、同一事件类型的声明：`None` 是不限 `#` 后面的部分。
    let allowed: Vec<Option<&str>> = declared
        .iter()
        .filter_map(|d| d.split_once(':'))
        .filter(|(p, _)| *p == prefix)
        .map(|(_, f)| split_filter(f))
        .filter(|(t, _)| *t == event_type)
        .map(|(_, k)| k)
        .collect();
    let keys: Vec<Option<&str>> = if allowed.contains(&None) {
        vec![key]
    } else if key.is_some() {
        allowed.into_iter().filter(|k| *k == key).collect()
    } else {
        allowed
    };
    keys.into_iter()
        .filter_map(|k| {
            let k = if prefix == SEND_STATE {
                Some(own_state_key(k, own)?)
            } else {
                k.map(str::to_owned)
            };
            Some(match k {
                Some(k) => format!("{prefix}:{event_type}#{k}"),
                None => format!("{prefix}:{event_type}"),
            })
        })
        .collect()
}

fn split_filter(filter: &str) -> (&str, Option<&str>) {
    match filter.split_once('#') {
        Some((event_type, key)) => (event_type, Some(key)),
        None => (filter, None),
    }
}

/// 发状态事件能用的 `state_key`：不限时收窄成自己的用户 ID，具体的要是自己的（见模块说明）。
fn own_state_key(key: Option<&str>, own: &UserId) -> Option<String> {
    let own = own.as_str();
    let Some(key) = key else {
        return Some(own.to_owned());
    };
    let mine = key == own
        || key
            .strip_prefix(own)
            .is_some_and(|rest| rest.starts_with('_'))
        || key
            .strip_prefix('_')
            .and_then(|rest| rest.strip_prefix(own))
            .is_some_and(|rest| rest.starts_with('_'));
    mine.then(|| key.to_owned())
}

#[cfg(test)]
mod tests;
