use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, bail};
use serde_json::json;
use wonton_protocol::{MAIL_FIELD, MemberStatus, PluginMail, PluginMailAttachment};

use super::api::Api;
use super::content::{
    ATTACHMENT_LIMIT, Body, BodySource, CONVERT_FAILED_KEY, Message, convert, received_ip,
};
use super::progress::{Decision, Progress, Store, Uploaded};
use crate::gateway::Gateway;
use crate::matrix::NoticeRoom;
use crate::plugin::Permission;
use crate::plugin::languages::Languages;

pub(super) struct Delivery {
    pub(super) api: Api,
    gateway: Arc<Gateway>,
    pub(super) progress: Store,
    rooms: BTreeMap<String, Arc<NoticeRoom>>,
}

impl Delivery {
    pub(super) fn new(api: Api, gateway: Arc<Gateway>, state_dir: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            api,
            gateway,
            progress: Store::open(&state_dir.join("mailpit/delivery"))?,
            rooms: BTreeMap::new(),
        })
    }

    pub(super) async fn scan(&mut self) {
        match self.api.list().await {
            Ok(ids) => {
                for id in ids {
                    self.attempt(&id).await;
                }
            }
            Err(error) if error.is_transport() => {
                tracing::debug!(%error, "Mailpit 还没连上，稍后再扫待投递的信");
            }
            Err(error) => tracing::error!(%error, "列出待投递信件失败"),
        }
    }

    pub(super) async fn attempt(&mut self, id: &str) {
        if let Err(error) = self.deliver(id).await {
            tracing::error!(%id, %error, "信件未投递完成，保留在 Mailpit");
        }
    }

    pub(super) async fn deliver(&mut self, id: &str) -> anyhow::Result<()> {
        let mut progress = self.progress.load(id)?;
        if matches!(progress.decision, Some(Decision::Discard { .. })) {
            return self.finish_discard(id, &progress).await;
        }
        let message: Message = serde_json::from_value(self.api.message(id).await?)?;
        if message.id != id {
            bail!("Mailpit 返回的信件 ID 与请求不一致");
        }
        if progress.decision.is_none() {
            progress.decision = Some(self.admit(id).await?);
            self.progress.save(id, &progress)?;
        }
        let (plugin_id, plugin_name) = match progress.decision.as_ref() {
            Some(Decision::Accepted {
                plugin_id,
                plugin_name,
            }) => (plugin_id.clone(), plugin_name.clone()),
            Some(Decision::Discard { .. }) => return self.finish_discard(id, &progress).await,
            None => bail!("信件尚未识别来源"),
        };
        let existing = self.recipients();
        let recipients = message.recipients(&existing);
        // 正文只算一次：转换在阻塞线程里跑并留进进度，重扫和重启都不再重复。
        let body = match progress.prepared.take() {
            Some(body) => body,
            None => {
                let body = self.prepare_body(&message).await;
                progress.prepared = Some(body.clone());
                self.progress.save(id, &progress)?;
                body
            }
        };
        let mut attachments = Vec::new();
        for part in message.attachments() {
            let attachment = part.attachment;
            if part.upload {
                self.upload_part(id, attachment, &mut progress).await?;
            }
            attachments.push(PluginMailAttachment {
                name: attachment.file_name.clone(),
                size: attachment.size,
                uploaded: part.upload,
            });
        }
        if body.attach_html && !message.html.is_empty() {
            let html = message.html.as_str();
            self.upload_html(id, html, &mut progress).await?;
            attachments.push(PluginMailAttachment {
                name: "message.html".into(),
                size: html.len() as u64,
                uploaded: html.len() as u64 <= ATTACHMENT_LIMIT,
            });
        }
        let mail = PluginMail {
            plugin_id,
            plugin_name,
            from: message
                .from
                .as_ref()
                .map_or("", |address| address.address.as_str())
                .to_owned(),
            subject: message.subject.clone(),
            body: body.text.clone(),
            attachments,
        };
        for username in recipients {
            if !progress.delivered.contains(username) {
                self.send_to(id, username, &mail, body.convert_failed, &progress.uploaded)
                    .await?;
                progress.delivered.insert(username.to_owned());
                self.progress.save(id, &progress)?;
            }
        }
        self.api.delete(id).await?;
        self.progress.remove(id)
    }

    fn failed_body(&self, message: &Message, truncated: bool) -> Body {
        let mut body = message.finish_body("", truncated);
        body.convert_failed = true;
        body
    }

    /// 正文：有纯文本直接用；只有 HTML 时把限长的一段交给阻塞线程转换，转不出来就留空待按语言填提示并附原文。
    async fn prepare_body(&self, message: &Message) -> Body {
        match message.body_source() {
            BodySource::Text(text) => message.finish_body(text, false),
            BodySource::Empty => message.finish_body("", false),
            BodySource::Html { html, truncated } => {
                let capped = html.clone();
                match tokio::task::spawn_blocking(move || convert(&capped)).await {
                    Ok(Ok(text)) => message.finish_body(&text, truncated),
                    Ok(Err(error)) => {
                        tracing::warn!(error = %error, "HTML 正文转不成文本，附原文继续投递");
                        self.failed_body(message, true)
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "转换邮件正文的线程挂了，附原文继续投递");
                        self.failed_body(message, true)
                    }
                }
            }
        }
    }

    async fn admit(&self, id: &str) -> anyhow::Result<Decision> {
        let headers = self.api.headers(id).await?;
        let source = received_ip(&headers)
            .ok()
            .and_then(|ip| self.gateway.plugins.source(ip));
        let Some(source) = source.filter(|source| source.permissions.contains(&Permission::Notify))
        else {
            return Ok(Decision::Discard {
                hint: Some(crate::l10n::text(
                    &self.gateway.plugins.owner_languages(),
                    "chat-mail-unknown-source",
                    &[],
                )),
            });
        };
        let name = self
            .gateway
            .plugins
            .title(&source.id)
            .as_ref()
            .map(crate::mcp::card_text)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| source.id.to_string());
        Ok(match self.gateway.notify_limiter.admit(&source.id) {
            Ok(()) => Decision::Accepted {
                plugin_id: source.id.to_string(),
                plugin_name: name,
            },
            Err(denied) => Decision::Discard {
                hint: crate::gateway::notify::limit_hint(
                    &name,
                    &denied,
                    &self.gateway.plugins.owner_languages(),
                ),
            },
        })
    }

    fn recipients(&self) -> BTreeMap<String, String> {
        let owner = self.gateway.owner.username();
        std::iter::once(owner.to_owned())
            .chain(
                self.gateway
                    .members
                    .list()
                    .into_iter()
                    .filter(|member| member.status == MemberStatus::Joined)
                    .map(|member| member.username),
            )
            .map(|username| (crate::owner::internal_email(&username), username))
            .collect()
    }

    async fn finish_discard(&self, id: &str, progress: &Progress) -> anyhow::Result<()> {
        if let Some(Decision::Discard { hint: Some(hint) }) = &progress.decision {
            let notices = self
                .gateway
                .notices
                .as_ref()
                .context("聊天服务不可用，无法提示丢弃邮件")?;
            notices
                .send_with_txn(
                    &json!({"msgtype": "m.notice", "body": hint}),
                    &format!("mail-{id}-discard"),
                )
                .await?;
        }
        self.api.delete(id).await?;
        self.progress.remove(id)
    }

    async fn upload_part(
        &self,
        id: &str,
        part: &super::content::Attachment,
        progress: &mut Progress,
    ) -> anyhow::Result<()> {
        if progress.uploaded.contains_key(&part.part_id) {
            return Ok(());
        }
        let matrix = self.gateway.matrix.as_ref().context("聊天服务不可用")?;
        let bytes = self.api.attachment(id, &part.part_id).await?;
        if bytes.len() as u64 != part.size || bytes.len() as u64 > ATTACHMENT_LIMIT {
            bail!("邮件附件大小与 Mailpit 元数据不一致或超过上限");
        }
        let uri = matrix
            .core_session()
            .upload_media(&part.content_type, &part.file_name, bytes)
            .await?;
        progress.uploaded.insert(
            part.part_id.clone(),
            Uploaded {
                name: part.file_name.clone(),
                content_type: part.content_type.clone(),
                size: part.size,
                uri,
            },
        );
        self.progress.save(id, progress)
    }

    async fn upload_html(
        &self,
        id: &str,
        html: &str,
        progress: &mut Progress,
    ) -> anyhow::Result<()> {
        if html.len() as u64 > ATTACHMENT_LIMIT || progress.uploaded.contains_key("original-html") {
            return Ok(());
        }
        let matrix = self.gateway.matrix.as_ref().context("聊天服务不可用")?;
        let uri = matrix
            .core_session()
            .upload_media("text/html", "message.html", html.as_bytes().to_vec())
            .await?;
        progress.uploaded.insert(
            "original-html".into(),
            Uploaded {
                name: "message.html".into(),
                content_type: "text/html".into(),
                size: html.len() as u64,
                uri,
            },
        );
        self.progress.save(id, progress)
    }

    async fn send_to(
        &mut self,
        id: &str,
        username: &str,
        mail: &PluginMail,
        convert_failed: bool,
        uploaded: &BTreeMap<String, Uploaded>,
    ) -> anyhow::Result<()> {
        let room = self.room(username)?;
        let text = if convert_failed {
            crate::l10n::text(&self.languages(username), CONVERT_FAILED_KEY, &[])
        } else {
            mail.body.clone()
        };
        let body = format!("{}：{}\n{}", mail.plugin_name, mail.subject, text);
        room.send_with_txn(
            &json!({"msgtype":"m.text", "body":body, MAIL_FIELD:mail}),
            &format!("mail-{id}-{username}-text"),
        )
        .await?;
        for (part, file) in uploaded {
            room.send_with_txn(
                &json!({
                    "msgtype":"m.file", "body":file.name, "filename":file.name, "url":file.uri,
                    "info":{"mimetype":file.content_type, "size":file.size},
                }),
                &format!("mail-{id}-{username}-file-{part}"),
            )
            .await?;
        }
        Ok(())
    }

    /// 收件人的语言；没记录就按主人的。
    fn languages(&self, username: &str) -> Languages {
        let header = self.gateway.plugins.person_languages(username);
        header
            .map(|header| Languages::from_header(Some(&header)))
            .unwrap_or_else(|| self.gateway.plugins.owner_languages())
    }

    fn room(&mut self, username: &str) -> anyhow::Result<Arc<NoticeRoom>> {
        if username == self.gateway.owner.username() {
            return self.gateway.notices.clone().context("聊天服务不可用");
        }
        if let Some(room) = self.rooms.get(username) {
            return Ok(room.clone());
        }
        let matrix = self.gateway.matrix.clone().context("聊天服务不可用")?;
        let state = self.gateway.plugins.data_dir().join("core");
        std::fs::create_dir_all(&state).context("建立家人通知房间记录目录")?;
        let plugins = self.gateway.plugins.clone();
        let person = username.to_owned();
        let room = Arc::new(NoticeRoom::new(
            matrix,
            username,
            state.join(format!("matrix-notice-room-{username}")),
            Arc::new(move || {
                crate::plugin::languages::Languages::from_header(
                    plugins.person_languages(&person).as_deref(),
                )
            }),
        ));
        self.rooms.insert(username.to_owned(), room.clone());
        Ok(room)
    }
}
