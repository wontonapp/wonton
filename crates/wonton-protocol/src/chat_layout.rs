//! 聊天页的显示方式（ADR-0060）：气泡或文档，外加几项只有少量预设的可调项。
//!
//! 客户端核心把插件声明的默认值、用户在账号数据里的选择合成每个房间的 [`ChatLayout`]，壳只按它渲染，不自己合并。
//! 插件只能选预设，带不进样式和代码。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 存用户选择的 Matrix 全局账号数据事件类型，内容是 [`ChatLayoutSettings`]。
pub const CHAT_LAYOUT_EVENT: &str = "app.wonton.chat_layout";

/// 聊天页的形态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ChatForm {
    /// 即时消息的气泡。
    #[default]
    Bubbles,
    /// 别人（多半是 Bot）的消息不装气泡、铺满宽度，像读文章；自己的话仍是气泡。
    Document,
    /// 比这个 App 新的取值，按 `Bubbles` 显示。
    #[serde(other)]
    Other,
}

/// 消息文字的字号，在 App 的字号（[`TextSize`](crate::TextSize)）上再加减一级。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ChatTextScale {
    Smaller,
    #[default]
    Standard,
    Larger,
    /// 比这个 App 新的取值，按 `Standard` 显示。
    #[serde(other)]
    Other,
}

/// 代码块和表格放不下时怎么办。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum CodeOverflow {
    /// 不折行，横向滚动。
    #[default]
    Scroll,
    /// 折行；表格每行画成一张小卡片。
    Wrap,
    /// 比这个 App 新的取值，按 `Scroll` 显示。
    #[serde(other)]
    Other,
}

/// 一个房间合成后的显示方式，壳照着画。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChatLayout {
    pub form: ChatForm,
    pub text_scale: ChatTextScale,
    pub code: CodeOverflow,
}

/// 用户改过的项；为空的项跟着默认值走（插件声明的，或全局默认）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChatLayoutChoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<ChatForm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_scale: Option<ChatTextScale>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeOverflow>,
}

/// 账号数据 [`CHAT_LAYOUT_EVENT`] 的内容。只存用户改过的：插件以后改了默认值，没动过的用户自动跟上。
///
/// 客户端核心改一项时按原始 JSON 只改那一处，其余原样写回：旧 App 读到不认识的取值会落到 `Other`，
/// 整份重写会把新 App 存的值冲掉。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatLayoutSettings {
    /// 群聊、和人的私聊，以及没有声明形态的 Bot 用的默认值。
    #[serde(default)]
    pub default: ChatLayoutChoice,
    /// 按 Bot 的 Matrix 用户 ID 覆盖。
    #[serde(default)]
    pub bots: BTreeMap<String, ChatLayoutChoice>,
}

/// 插件的 Bot 在聊天页上的声明（清单 `x-wonton.bot.chat_layout`），由家里的服务器随插件列表给出。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginChat {
    /// 插件声明的默认形态；没声明是 `Bubbles`。
    pub form: ChatForm,
}

/// Bot 资料页“显示”一节用的：默认值和用户改过的项。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotChatSettings {
    /// 插件声明的默认值合进全局默认之后的结果，“恢复默认”就回到它。
    pub default: ChatLayoutChoice,
    /// 用户对这个 Bot 改过的项。
    pub choice: ChatLayoutChoice,
}

/// [`ChatAction::SetChatLayout`](crate::ChatAction::SetChatLayout) 改哪一份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatLayoutTarget {
    /// 全局默认（[`ChatLayoutSettings::default`]）。
    Default,
    /// 这个 Bot 的覆盖。
    Bot { user_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_values_fall_back_to_other() {
        let choice: ChatLayoutChoice =
            serde_json::from_value(serde_json::json!({"form": "hologram", "code": "fold"}))
                .unwrap();
        assert_eq!(choice.form, Some(ChatForm::Other));
        assert_eq!(choice.code, Some(CodeOverflow::Other));
        assert_eq!(choice.text_scale, None);
    }

    #[test]
    fn settings_only_store_changed_items() {
        let mut settings = ChatLayoutSettings::default();
        settings.bots.insert(
            "@codex:home.example".to_owned(),
            ChatLayoutChoice {
                form: Some(ChatForm::Document),
                ..ChatLayoutChoice::default()
            },
        );
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            serde_json::json!({"default": {}, "bots": {"@codex:home.example": {"form": "document"}}})
        );
    }

    #[test]
    fn missing_settings_fields_are_empty() {
        let settings: ChatLayoutSettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(settings, ChatLayoutSettings::default());
    }
}
