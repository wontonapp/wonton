use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use wonton_protocol::BotRef;

/// 字符串保持插件级选择；带判别字段的对象绑定提供方 Bot，未知形状原样保留。
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Binding {
    Plugin(String),
    Bot(BotRef),
    Other(serde_json::Value),
}

impl Binding {
    pub(super) fn provider(&self) -> Option<&str> {
        match self {
            Self::Plugin(id) => Some(id),
            Self::Bot(bot) => Some(&bot.plugin_id),
            Self::Other(_) => None,
        }
    }

    pub(super) fn bot(&self) -> Option<&str> {
        match self {
            Self::Bot(bot) => Some(&bot.bot),
            Self::Plugin(_) | Self::Other(_) => None,
        }
    }
}

impl Serialize for Binding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Plugin(id) => id.serialize(serializer),
            Self::Bot(bot) => {
                let mut value = serializer.serialize_struct("Binding", 3)?;
                value.serialize_field("kind", "bot")?;
                value.serialize_field("plugin_id", &bot.plugin_id)?;
                value.serialize_field("bot", &bot.bot)?;
                value.end()
            }
            Self::Other(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let Some(id) = value.as_str() {
            return Ok(Self::Plugin(id.to_owned()));
        }
        if value.get("kind").and_then(serde_json::Value::as_str) == Some("bot")
            && let (Some(plugin_id), Some(bot)) = (
                value.get("plugin_id").and_then(serde_json::Value::as_str),
                value.get("bot").and_then(serde_json::Value::as_str),
            )
        {
            return Ok(Self::Bot(BotRef {
                plugin_id: plugin_id.to_owned(),
                bot: bot.to_owned(),
            }));
        }
        Ok(Self::Other(value))
    }
}

pub(super) fn consumer_key(consumer: &str, bot: &str) -> String {
    format!("{consumer}/{bot}")
}

pub(super) fn credential_name<'a>(
    consumer: &'a str,
    bot: Option<&str>,
    provider_bot: Option<&str>,
) -> std::borrow::Cow<'a, str> {
    match (bot, provider_bot) {
        (None, None) => consumer.into(),
        (Some(bot), None) => consumer_key(consumer, bot).into(),
        (bot, Some(target)) => format!("{consumer}/{}/{target}", bot.unwrap_or_default()).into(),
    }
}
