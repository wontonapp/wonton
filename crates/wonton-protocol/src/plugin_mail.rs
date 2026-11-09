//! 核心转进收件人“Wonton”私聊的应用邮件（ADR-0056）。
//!
//! 核心以核心账号发送 `m.text`，结构化内容放在 [`MAIL_FIELD`]，消息的纯文本 `body` 留给其他客户端。
//! 客户端只信任核心账号发来的合法邮件，推送归入
//! [`NotificationCategory::PluginNotice`](crate::NotificationCategory::PluginNotice)。

use serde::{Deserialize, Serialize};

/// 消息内容里放应用邮件的字段。
pub const MAIL_FIELD: &str = "app.wonton.plugin_mail";

/// 核心转发的一封应用邮件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginMail {
    pub plugin_id: String,
    pub plugin_name: String,
    /// 邮件的发件地址，不用于认定插件来源。
    pub from: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<PluginMailAttachment>,
}

/// 邮件附件；未上传的附件仍保留在列表里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginMailAttachment {
    pub name: String,
    /// 附件大小，单位为字节。
    pub size: u64,
    /// 是否已经作为 `m.file` 上传到聊天。
    pub uploaded: bool,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn mail_preserves_the_wire_fields_and_attachment_metadata() {
        let content = json!({
            "app.wonton.plugin_mail": {
                "plugin_id": "app.example.documents",
                "plugin_name": "Documents",
                "from": "documents@plugins.wonton.internal",
                "subject": "Export ready",
                "body": "Your export is ready.",
                "attachments": [
                    {"name": "receipt.pdf", "size": 1024, "uploaded": true},
                    {"name": "archive.zip", "size": 4_294_967_296_u64, "uploaded": false},
                ],
            },
        });
        let mail: PluginMail = serde_json::from_value(content[MAIL_FIELD].clone()).unwrap();
        assert_eq!(
            mail.attachments,
            vec![
                PluginMailAttachment {
                    name: "receipt.pdf".into(),
                    size: 1024,
                    uploaded: true,
                },
                PluginMailAttachment {
                    name: "archive.zip".into(),
                    size: 4_294_967_296,
                    uploaded: false,
                },
            ]
        );
        assert_eq!(serde_json::to_value(mail).unwrap(), content[MAIL_FIELD]);
    }
}
