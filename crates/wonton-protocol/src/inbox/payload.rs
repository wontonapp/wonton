use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::choice::ChoicePayload;
use super::confirm::ConfirmPayload;
use super::notice::NoticePayload;
use super::page::PagePayload;
use super::recap::RecapPayload;

#[cfg(feature = "uniffi")]
uniffi::custom_type!(Value, String, {
    remote,
    lower: |value| value.to_string(),
    try_lift: |text| serde_json::from_str(&text).map_err(Into::into),
});

/// 条目载荷。`type` 和条目的类型必须一致。
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum InboxPayload {
    Choice(ChoicePayload),
    Page(PagePayload),
    Recap(RecapPayload),
    Confirm(ConfirmPayload),
    Notice(NoticePayload),
    /// 更新的核心给出的类型：wire 上只发 `{"type":"other"}`，原文放在这里和 [`InboxItem::raw_payload`]，
    /// 旧壳按单元变体也解析得了。
    Other(Value),
}

#[derive(Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum KnownPayload<'a> {
    Choice(&'a ChoicePayload),
    Page(&'a PagePayload),
    Recap(&'a RecapPayload),
    Confirm(&'a ConfirmPayload),
    Notice(&'a NoticePayload),
}

impl Serialize for InboxPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let known = match self {
            Self::Choice(value) => KnownPayload::Choice(value),
            Self::Page(value) => KnownPayload::Page(value),
            Self::Recap(value) => KnownPayload::Recap(value),
            Self::Confirm(value) => KnownPayload::Confirm(value),
            Self::Notice(value) => KnownPayload::Notice(value),
            Self::Other(_) => return serde_json::json!({"type": "other"}).serialize(serializer),
        };
        known.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for InboxPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload = value.get("payload").cloned().unwrap_or(Value::Null);
        let parsed = match kind {
            "choice" => serde_json::from_value(payload).map(Self::Choice),
            "page" => serde_json::from_value(payload).map(Self::Page),
            "recap" => serde_json::from_value(payload).map(Self::Recap),
            "confirm" => serde_json::from_value(payload).map(Self::Confirm),
            "notice" => serde_json::from_value(payload).map(Self::Notice),
            _ => return Ok(Self::Other(payload)),
        };
        parsed.map_err(serde::de::Error::custom)
    }
}
