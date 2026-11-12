use std::{collections::HashSet, fmt};

use wonton_matrix::{FileContent, ImageContent, Message, MessageBody, OwnedUserId, UserId};

/// 一条主人消息：文字、附件，或桥不认识的内容。
#[derive(Clone)]
pub struct Incoming {
    /// 这条消息的事件 ID；撤回（如转交登录密钥后）要用它。
    pub event_id: String,
    /// 所属话题的根消息事件 ID；在主对话里发的消息是 `None`。
    pub thread_root: Option<String>,
    /// 服务器分配的发送时间（Unix 毫秒）；重启后拿它和已读位置比，分辨停机期间的积压。
    pub timestamp_ms: u64,
    pub content: IncomingContent,
}

#[derive(Clone)]
pub enum IncomingContent {
    Text(String),
    /// 图片或文件；媒体句柄要等发给 Agent 时才解出，不写日志。
    Media(MediaAttachment),
    /// 桥不转发的消息类型（语音、视频、贴纸等）；`kind` 是给提示文案的分类：
    /// `audio`、`video` 或其余的 `other`。
    Unsupported {
        kind: String,
    },
}

/// 一条待转发的附件：元数据加媒体句柄。句柄是 JSON 编码的媒体来源，加密房间里的带着
/// 解密密钥，跟密钥一样保管，所以 [`Debug`] 不打印它。
#[derive(Clone)]
pub struct MediaAttachment {
    pub filename: String,
    pub mime_type: Option<String>,
    /// 事件里声明的字节数，发送方可以不声明或声明不实；下载后再按实际字节数复核。
    pub declared_size: Option<u64>,
    /// 附件自带的说明文字（图片或文件的 caption）。
    pub caption: Option<String>,
    /// wonton-matrix 的媒体句柄，凭 [`crate::room::Room::download_media`] 下载并解密。
    pub token: String,
}

impl fmt::Debug for MediaAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaAttachment")
            .field("filename", &self.filename)
            .field("mime_type", &self.mime_type)
            .field("declared_size", &self.declared_size)
            .field("caption", &self.caption)
            .field("token", &"…")
            .finish()
    }
}

impl MediaAttachment {
    fn from_image(image: &ImageContent) -> Self {
        Self {
            filename: image.filename.clone(),
            mime_type: image.mime_type.clone(),
            declared_size: image.size,
            caption: image.caption.clone(),
            token: image.media_token(),
        }
    }

    fn from_file(file: &FileContent) -> Self {
        Self {
            filename: file.filename.clone(),
            mime_type: file.mime_type.clone(),
            declared_size: file.size,
            caption: file.caption.clone(),
            token: file.media_token(),
        }
    }
}

pub fn new_allowed_messages(
    messages: &[Message],
    allowed_users: &[OwnedUserId],
    bot: &UserId,
    group: bool,
    seen: &mut HashSet<String>,
) -> Vec<Incoming> {
    messages
        .iter()
        .filter(|message| !message.is_own && allowed_users.contains(&message.sender))
        .filter_map(|message| {
            // 密钥可能晚于事件到达；正文解密后的同一事件仍需交给 Agent。
            if matches!(message.body, MessageBody::Undecryptable) {
                return None;
            }
            // 撤回不是输入；同一事件迟到的重投由 seen 挡住。
            if matches!(message.body, MessageBody::Redacted) {
                return None;
            }
            let event_id = message.event_id.as_ref()?.to_string();
            let thread_root = message.thread_root.as_ref().map(ToString::to_string);
            if !seen.insert(event_id.clone()) {
                return None;
            }
            let content = match &message.body {
                MessageBody::Text(text) => {
                    // 群里有别的 Bot，命令要由主人点名本 Bot；私聊不用。
                    if group
                        && text.trim_start().starts_with('/')
                        && !message.mentions.iter().any(|mentioned| mentioned == bot)
                    {
                        return None;
                    }
                    IncomingContent::Text(text.clone())
                }
                MessageBody::Image(image) => {
                    IncomingContent::Media(MediaAttachment::from_image(image))
                }
                MessageBody::File(file) => IncomingContent::Media(MediaAttachment::from_file(file)),
                MessageBody::Notice(_) | MessageBody::Other { .. } => {
                    IncomingContent::Unsupported {
                        kind: unsupported_kind(&message.body).to_owned(),
                    }
                }
                MessageBody::Undecryptable | MessageBody::Redacted => return None,
            };
            Some(Incoming {
                event_id,
                thread_root,
                timestamp_ms: message.timestamp_ms,
                content,
            })
        })
        .collect()
}

/// 给“不支持”提示分类用的短标签：已知类型给人话，其余归为一类。
pub fn unsupported_kind(body: &MessageBody) -> &'static str {
    match body {
        MessageBody::Other { kind } if kind == "m.audio" => "audio",
        MessageBody::Other { kind } if kind == "m.video" => "video",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_matrix::{Message, OwnedUserId, SendStatus};

    fn owner_message(event_id: &str, text: &str, mentions: &[&str]) -> Message {
        Message {
            event_id: Some(event_id.parse().unwrap()),
            sender: "@owner:example.org".parse().unwrap(),
            sender_name: String::new(),
            is_own: false,
            timestamp_ms: 1,
            status: SendStatus::Sent,
            body: MessageBody::Text(text.into()),
            custom: Default::default(),
            mentions: mentions.iter().map(|user| user.parse().unwrap()).collect(),
            agent_run: None,
            thread_root: None,
            edited: false,
            read_by: Vec::new(),
            reply: None,
            group: None,
            can_redact: false,
        }
    }

    #[test]
    fn delayed_decryption_delivers_the_allowed_message_once() {
        let owner: OwnedUserId = "@owner:example.org".parse().unwrap();
        let bot: OwnedUserId = "@bot:example.org".parse().unwrap();
        let allowed = [owner.clone()];
        let mut message = owner_message("$delayed:example.org", "", &[]);
        message.body = MessageBody::Undecryptable;
        let mut seen = HashSet::new();
        assert!(
            new_allowed_messages(&[message.clone()], &allowed, &bot, false, &mut seen).is_empty()
        );
        message.body = MessageBody::Text("hello".into());
        let incoming = new_allowed_messages(&[message.clone()], &allowed, &bot, false, &mut seen);
        let IncomingContent::Text(text) = &incoming[0].content else {
            panic!("应是文字");
        };
        assert_eq!(text, "hello");
        assert_eq!(incoming[0].event_id, "$delayed:example.org");
        assert_eq!(incoming.len(), 1);
        assert!(new_allowed_messages(&[message], &allowed, &bot, false, &mut seen).is_empty());
        // 名单外的人说话不交给 Agent。
        let mut stranger = owner_message("$stranger:example.org", "hello", &[]);
        stranger.sender = "@stranger:example.org".parse().unwrap();
        assert!(new_allowed_messages(&[stranger], &allowed, &bot, false, &mut seen).is_empty());
    }

    #[test]
    fn a_group_only_takes_commands_that_mention_the_bot() {
        let owner: OwnedUserId = "@owner:example.org".parse().unwrap();
        let bot: OwnedUserId = "@bot:example.org".parse().unwrap();
        let allowed = [owner.clone()];
        let plain = owner_message("$plain:example.org", "普通消息", &[]);
        let command = owner_message("$command:example.org", "/review", &[]);
        let mentioned = owner_message("$mentioned:example.org", "/review", &["@bot:example.org"]);
        let mut seen = HashSet::new();
        let incoming = new_allowed_messages(
            &[plain.clone(), command, mentioned],
            &allowed,
            &bot,
            true,
            &mut seen,
        );
        assert_eq!(
            incoming
                .iter()
                .map(|incoming| incoming.event_id.as_str())
                .collect::<Vec<_>>(),
            ["$plain:example.org", "$mentioned:example.org"]
        );

        // 私聊不分提及。
        let mut seen = HashSet::new();
        assert_eq!(
            new_allowed_messages(
                &[plain, owner_message("$c2:example.org", "/review", &[])],
                &allowed,
                &bot,
                false,
                &mut seen
            )
            .len(),
            2
        );
    }

    fn image_message(event_id: &str) -> Message {
        let mut message = owner_message(event_id, "", &[]);
        message.body = MessageBody::Image(ImageContent::for_test(
            "照片.jpg",
            Some("image/jpeg"),
            Some(7),
            Some("这是说明"),
            "mxc://example.org/photo",
        ));
        message
    }

    #[test]
    fn an_image_becomes_a_media_attachment_with_caption_and_handle() {
        let owner: OwnedUserId = "@owner:example.org".parse().unwrap();
        let bot: OwnedUserId = "@bot:example.org".parse().unwrap();
        let allowed = [owner];
        let mut seen = HashSet::new();
        let incoming = new_allowed_messages(
            &[image_message("$img:example.org")],
            &allowed,
            &bot,
            false,
            &mut seen,
        );
        let IncomingContent::Media(media) = &incoming[0].content else {
            panic!("应是媒体");
        };
        assert_eq!(media.filename, "照片.jpg");
        assert_eq!(media.mime_type.as_deref(), Some("image/jpeg"));
        assert_eq!(media.declared_size, Some(7));
        assert_eq!(media.caption.as_deref(), Some("这是说明"));
        assert!(media.token.contains("mxc://example.org/photo"));
        // 句柄带着解密密钥，Debug 不打印。
        assert!(!format!("{media:?}").contains("mxc"));
    }

    #[test]
    fn a_file_becomes_a_media_attachment_and_audio_is_unsupported() {
        let owner: OwnedUserId = "@owner:example.org".parse().unwrap();
        let bot: OwnedUserId = "@bot:example.org".parse().unwrap();
        let allowed = [owner];
        let mut file = owner_message("$file:example.org", "", &[]);
        file.body = MessageBody::File(FileContent::for_test(
            "报表.pdf",
            Some("application/pdf"),
            Some(9),
            None,
            "mxc://example.org/report",
        ));
        let mut audio = owner_message("$audio:example.org", "", &[]);
        audio.body = MessageBody::Other {
            kind: "m.audio".to_owned(),
        };
        let mut redacted = owner_message("$gone:example.org", "", &[]);
        redacted.body = MessageBody::Redacted;
        let mut seen = HashSet::new();
        let incoming =
            new_allowed_messages(&[file, audio, redacted], &allowed, &bot, false, &mut seen);
        assert_eq!(incoming.len(), 2);
        let IncomingContent::Media(media) = &incoming[0].content else {
            panic!("应是媒体");
        };
        assert_eq!(media.filename, "报表.pdf");
        assert!(media.caption.is_none());
        let IncomingContent::Unsupported { kind } = &incoming[1].content else {
            panic!("应是不支持");
        };
        assert_eq!(
            unsupported_kind(&MessageBody::Other {
                kind: "m.audio".to_owned()
            }),
            "audio"
        );
        assert_eq!(
            unsupported_kind(&MessageBody::Other {
                kind: "m.location".to_owned()
            }),
            "other"
        );
        assert_eq!(kind, "audio");
    }
}
