//! 房间输入框里的草稿。

use matrix_sdk::{
    ComposerDraft, ComposerDraftType,
    ruma::{EventId, RoomId},
};

use crate::{Draft, MatrixSession, Result};

impl MatrixSession {
    pub async fn draft(&self, room_id: &RoomId, thread: Option<&EventId>) -> Result<Option<Draft>> {
        let room = self.joined(room_id)?;
        Ok(room.load_composer_draft(thread).await?.map(|draft| Draft {
            body: draft.plain_text,
            reply_to: match draft.draft_type {
                ComposerDraftType::Reply { event_id } => Some(event_id),
                ComposerDraftType::NewMessage | ComposerDraftType::Edit { .. } => None,
            },
        }))
    }
    pub async fn save_draft(
        &self,
        room_id: &RoomId,
        thread: Option<&EventId>,
        draft: &Draft,
    ) -> Result<()> {
        let room = self.joined(room_id)?;
        if draft.body.is_empty() && draft.reply_to.is_none() {
            room.clear_composer_draft(thread).await?;
        } else {
            let draft = ComposerDraft {
                plain_text: draft.body.clone(),
                // 不发格式化内容，正文就是原文。
                html_text: None,
                draft_type: match &draft.reply_to {
                    Some(event_id) => ComposerDraftType::Reply {
                        event_id: event_id.clone(),
                    },
                    None => ComposerDraftType::NewMessage,
                },
                attachments: Vec::new(),
            };
            room.save_composer_draft(draft, thread).await?;
        }
        Ok(())
    }
}
