//! 输入框的 @ 候选：群里在场的人，Bot 排在前面。私聊里全是对方，不列。
//!
//! 候选跟着房间成员和 Bot 名单补，一个房间只起一条跟踪任务；它只读本机快照，不碰网络。

use std::collections::HashSet;
use std::sync::Arc;

use wonton_matrix::ChatRoom;
use wonton_protocol::{BotView, ComposerMention, MediaHandle};

use super::{TimelineKey, lock};
use crate::scope::Scope;

/// 起跟踪：成员、显示名或 Bot 名单一变就重算候选推到输入框。
pub(crate) fn watch(scope: &Arc<Scope>, room_id: &str) {
    let chat = scope.chat.clone();
    {
        let mut watched = lock(&chat.mentions_watched);
        if !watched.insert(room_id.to_owned()) {
            return;
        }
    }
    let Ok(id) = room_id.parse() else {
        return;
    };
    let key = TimelineKey::main(id);
    let scope = scope.clone();
    let room_id = room_id.to_owned();
    let spawned = scope.clone();
    let mut bots_updates = chat.bots.view.subscribe();
    let mut session = chat.session_out.subscribe();
    spawned.spawn(async move {
        loop {
            match chat.room(&key).await {
                Ok(room) => {
                    let bots = chat.bots.view.borrow().bots.clone();
                    let candidates = build(&room, &bots);
                    scope.composer.set_mentions(&room_id, candidates);
                    tokio::select! {
                        () = room.member_names_changed() => {}
                        result = bots_updates.changed() => {
                            if result.is_err() {
                                return;
                            }
                        }
                    }
                }
                Err(_) => {
                    scope.composer.set_mentions(&room_id, Vec::new());
                    if session.changed().await.is_err() {
                        return;
                    }
                }
            }
        }
    });
}

fn build(room: &ChatRoom, bots: &[BotView]) -> Vec<ComposerMention> {
    let watched = room.mention_members();
    if watched.is_empty() {
        return Vec::new();
    }
    let chat_bots: HashSet<&str> = bots.iter().map(|bot| bot.user_id.as_str()).collect();
    let mut candidates: Vec<ComposerMention> = watched
        .into_iter()
        .map(|member| ComposerMention {
            is_bot: chat_bots.contains(member.user_id.as_str()),
            avatar: member.avatar.as_deref().and_then(|url| {
                wonton_matrix::media_token(url)
                    .ok()
                    .map(|token| MediaHandle { token })
            }),
            user_id: member.user_id,
            display_name: member.display_name,
        })
        .collect();
    candidates.sort_by(|left, right| {
        right
            .is_bot
            .cmp(&left.is_bot)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    candidates
}
