//! App 的链接（`wonton://…`）：桌面快捷方式、快捷指令和 Siri 用它打开 App 里的某一处。格式只在这里定，两端都经核心
//! 编解码，不自己拼。

use serde::{Deserialize, Serialize};

use crate::TabRef;

const SCHEME: &str = "wonton://";

/// App 里能直接打开的地方。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppLink {
    /// 打开一个页面；没固定在底栏上的插件页盖在主界面上打开（[`Action::SelectTab`](crate::Action::SelectTab)）。
    Tab { tab: TabRef },
    /// 新会话：打开“发起私聊”。
    NewChat,
    /// 打开和默认 Agent 的聊天。带一句话直接发出去的“问 Bot”用 [`Action::AskAgent`](crate::Action::AskAgent)。
    AgentChat,
    /// 打开设置里的“其他服务器”，滚到“请求”（ADR-0050：核心在“Wonton”私聊里的配对提醒带这个链接）。
    Peers,
}

impl AppLink {
    pub fn to_url(&self) -> String {
        let path = match self {
            Self::Tab { tab } => match tab {
                TabRef::Chat => "tab/chat".to_owned(),
                TabRef::Apps => "tab/apps".to_owned(),
                TabRef::Settings => "tab/settings".to_owned(),
                TabRef::Plugin {
                    plugin_id,
                    page: None,
                } => format!("tab/plugin/{plugin_id}"),
                TabRef::Plugin {
                    plugin_id,
                    page: Some(page),
                } => format!("tab/plugin/{plugin_id}/{page}"),
            },
            Self::NewChat => "new-chat".to_owned(),
            Self::AgentChat => "agent-chat".to_owned(),
            Self::Peers => "settings/peers".to_owned(),
        };
        format!("{SCHEME}{path}")
    }

    /// 认不出的链接（比如新版 App 生成、旧版收到的）是 `None`：壳照常打开 App，不报错。
    pub fn parse(url: &str) -> Option<Self> {
        let path = url.strip_prefix(SCHEME)?;
        let parts: Vec<&str> = path.split('/').collect();
        let tab = |tab| Some(Self::Tab { tab });
        match parts.as_slice() {
            ["tab", "chat"] => tab(TabRef::Chat),
            ["tab", "apps"] => tab(TabRef::Apps),
            ["tab", "settings"] => tab(TabRef::Settings),
            ["tab", "plugin", plugin_id] if is_id(plugin_id) => tab(TabRef::Plugin {
                plugin_id: (*plugin_id).to_owned(),
                page: None,
            }),
            ["tab", "plugin", plugin_id, page] if is_id(plugin_id) && is_id(page) => {
                tab(TabRef::Plugin {
                    plugin_id: (*plugin_id).to_owned(),
                    page: Some((*page).to_owned()),
                })
            }
            ["new-chat"] => Some(Self::NewChat),
            ["agent-chat"] => Some(Self::AgentChat),
            ["settings", "peers"] => Some(Self::Peers),
            _ => None,
        }
    }
}

/// 插件 ID 和页面 ID 都是清单里的短名：字母或数字开头，只有字母、数字、`-`、`_` 和 `.`。别的写法（比如 `..`）
/// 说明链接被改过。
fn is_id(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_alphanumeric())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Action;

    #[test]
    fn asking_the_agent_round_trips_through_json() {
        let action = Action::AskAgent {
            text: "明天几点下雨？".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(serde_json::from_str::<Action>(&json).unwrap(), action);
    }

    #[test]
    fn every_link_round_trips_through_its_url() {
        let links = [
            AppLink::Tab { tab: TabRef::Chat },
            AppLink::Tab { tab: TabRef::Apps },
            AppLink::Tab {
                tab: TabRef::Settings,
            },
            AppLink::Tab {
                tab: TabRef::Plugin {
                    plugin_id: "filebrowser".into(),
                    page: None,
                },
            },
            AppLink::Tab {
                tab: TabRef::Plugin {
                    plugin_id: "grafana".into(),
                    page: Some("explore".into()),
                },
            },
            AppLink::NewChat,
            AppLink::AgentChat,
            AppLink::Peers,
        ];
        for link in links {
            assert_eq!(AppLink::parse(&link.to_url()), Some(link));
        }
    }

    #[test]
    fn unknown_or_tampered_links_are_not_opened() {
        for url in [
            "https://example.com/tab/chat",
            "wonton://tab/unknown",
            "wonton://tab/plugin/",
            "wonton://tab/plugin/../etc",
            "wonton://tab/plugin/a b",
            "wonton://tab/plugin/a/b/c",
            "wonton://ask",
        ] {
            assert_eq!(AppLink::parse(url), None, "{url}");
        }
    }
}
