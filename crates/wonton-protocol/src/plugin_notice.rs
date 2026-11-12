//! 插件发给主人的通知（ADR-0047）。
//!
//! 插件经 `notify.wonton.internal` 交给核心，核心以核心账号（[`CORE_LOCALPART`](crate::CORE_LOCALPART)）发进主人的
//! “Wonton”私聊：结构化内容放在消息内容的 [`NOTICE_FIELD`] 字段，`body` 是给其他客户端看的纯文本。`info` 是
//! `m.notice`，不推送；`warning`、`critical` 是 `m.text`，推送时类别是
//! [`NotificationCategory::PluginNotice`](crate::NotificationCategory::PluginNotice)。客户端只认核心账号发的，别人发的
//! 同样结构的内容当普通消息。

use serde::{Deserialize, Serialize};

/// 消息内容里放插件通知的字段。
pub const NOTICE_FIELD: &str = "app.wonton.plugin_notice";

/// 一条插件通知。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginNotice {
    pub plugin_id: String,
    /// 发出时插件的显示名。
    pub plugin_name: String,
    pub level: NoticeLevel,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// 插件网页里的一页（以 `/` 开头，可带 query），点通知打开插件的这一页。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// 通知的级别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    /// 只进聊天，不推送。
    Info,
    Warning,
    Critical,
    /// 不认识的级别：核心比 App 新。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_notice_reads_the_wire_shape_and_tolerates_new_levels() {
        let notice: PluginNotice = serde_json::from_value(json!({
            "plugin_id": "app.example.monitor",
            "plugin_name": "Monitor",
            "level": "critical",
            "title": "磁盘快满了",
            "path": "/hosts/1?tab=disk",
        }))
        .unwrap();
        assert_eq!(notice.level, NoticeLevel::Critical);
        assert_eq!(notice.body, None);
        assert_eq!(notice.path.as_deref(), Some("/hosts/1?tab=disk"));
        let back = serde_json::to_value(&notice).unwrap();
        assert!(back.get("body").is_none());

        let newer: NoticeLevel = serde_json::from_value(json!("emergency")).unwrap();
        assert_eq!(newer, NoticeLevel::Other);
    }
}
