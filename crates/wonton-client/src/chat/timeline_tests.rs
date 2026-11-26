use wonton_matrix::{Message, MessageBody, SendStatus, TimelineEntry, Vector};
use wonton_protocol::{
    BotRef, BotView, ChatViewModel, MessageContent, RichLinkKind, TimelineContent, TimelineDiff,
};

use crate::chat::agent_run::BotsView;
use crate::chat::links::LinkTargets;
use crate::chat::previews::Previews;
use crate::chat::read_mark::ReadMark;

use super::diffs::relinked;
use super::rebuild::move_read_mark;
use super::recompute::room_bots;

fn text_entry(id: &str, body: &str) -> TimelineEntry {
    TimelineEntry {
        id: id.to_owned(),
        kind: wonton_matrix::EntryKind::Message(Box::new(Message {
            event_id: None,
            sender: "@bot:wonton.localhost".parse().unwrap(),
            sender_name: "Bot".to_owned(),
            is_own: false,
            timestamp_ms: 0,
            status: SendStatus::Sent,
            body: MessageBody::Text(body.to_owned()),
            custom: Default::default(),
            mentions: Vec::new(),
            agent_run: None,
            thread_root: None,
            thread_replies: None,
            edited: false,
            read_by: Vec::new(),
            reply: None,
            group: None,
            can_redact: false,
            reactions: Vec::new(),
            is_direct: false,
            mentions_me: false,
            sender_avatar: None,
        })),
    }
}

fn kind(diff: &TimelineDiff) -> RichLinkKind {
    let TimelineDiff::Set { item, .. } = diff else {
        panic!("{diff:?}");
    };
    let TimelineContent::Message { message } = &item.content else {
        panic!("文字还是消息");
    };
    let MessageContent::Text { blocks, .. } = &message.body else {
        panic!("文字还是文字");
    };
    blocks
        .iter()
        .flat_map(|block| &block.spans)
        .find_map(|span| span.link.as_ref().map(|link| link.kind.clone()))
        .expect("链接还在")
}

/// 行为：先把插件网址渲染成外部网页，插件装好以后同一段文字重算成插件页。
#[test]
fn an_arriving_plugin_reclassifies_links() {
    let read = ReadMark::default();
    let entries: Vector<TimelineEntry> = vec![text_entry(
        "1",
        "打开 http://app-grafana.home.example:18480/",
    )]
    .into_iter()
    .collect();

    let before = relinked(
        &entries,
        &read,
        &LinkTargets::default(),
        &BotsView::default(),
        &Previews::new(),
    );
    assert_eq!(kind(&before[0]), RichLinkKind::External);

    let links = super::super::links::test_targets(&[(
        "app.grafana",
        "http://app-grafana.home.example:18480/",
    )]);
    let after = relinked(
        &entries,
        &read,
        &links,
        &BotsView::default(),
        &Previews::new(),
    );
    assert_eq!(
        kind(&after[0]),
        RichLinkKind::Plugin {
            plugin_id: "app.grafana".to_owned()
        }
    );
}

fn own_entry(id: &str) -> TimelineEntry {
    let mut entry = text_entry(id, id);
    if let wonton_matrix::EntryKind::Message(message) = &mut entry.kind {
        message.is_own = true;
    }
    entry
}

fn receipt_sets(diffs: &[TimelineDiff]) -> Vec<(u32, Option<wonton_protocol::MessageReceipt>)> {
    diffs
        .iter()
        .map(|diff| {
            let TimelineDiff::Set { index, item } = diff else {
                panic!("{diff:?}");
            };
            let TimelineContent::Message { message } = &item.content else {
                panic!("还是消息");
            };
            (*index, message.receipt)
        })
        .collect()
}

/// 行为：又发出一条以后，“已送达”从上一条挪到新的这条，两条各重发一次，别的不动。
#[test]
fn sending_another_message_moves_delivered_to_it() {
    let mut entries: Vector<TimelineEntry> = vec![text_entry("0", "hi"), own_entry("1")]
        .into_iter()
        .collect();
    let mut read = ReadMark::of(&entries);
    entries.push_back(own_entry("2"));

    let diffs = move_read_mark(
        &entries,
        &mut read,
        &LinkTargets::default(),
        &BotsView::default(),
        &Previews::new(),
    );

    assert_eq!(
        receipt_sets(&diffs),
        vec![
            (1, None),
            (2, Some(wonton_protocol::MessageReceipt::Delivered)),
        ]
    );
}

/// 行为：成员没取全、或插件的已安装清单没拿到（认识的 Bot 集合是空的）时，「在场的 AI」都算不准，
/// 如实给不确定；两样都齐了才给名单，群里一个认识的 Bot 都没有就是空名单。
#[test]
fn room_bots_are_uncertain_until_members_and_the_bot_list_are_ready() {
    let view = chat_view(&["@hermes:home.example"]);
    let ready = BotsView::of(&view, &[], true);
    let not_ready = BotsView::of(&view, &[], false);
    let members = || {
        vec![
            "@hermes:home.example".to_owned(),
            "@owner:home.example".to_owned(),
        ]
    };

    assert_eq!(room_bots(true, &not_ready, members), None);
    assert_eq!(room_bots(false, &ready, members), None);
    assert_eq!(
        room_bots(true, &ready, members),
        Some(vec!["@hermes:home.example".to_owned()])
    );
}

/// 一个认得 `bots` 里那些用户 ID 的聊天状态；别的字段取空值，这个测试只看 Bot 清单。
fn chat_view(bots: &[&str]) -> ChatViewModel {
    ChatViewModel {
        ask: None,
        chat_layout_default: Default::default(),
        account: wonton_protocol::ChatAccount::SignedOut,
        sync: wonton_protocol::ChatSync::Idle,
        encryption: wonton_protocol::EncryptionStatus {
            verification: wonton_protocol::DeviceVerification::Unknown,
            recovery: wonton_protocol::RecoveryStatus::Unknown,
            recovery_key: None,
            backup_reminder: None,
            can_verify_with_other_device: false,
            can_reset: false,
            recovery_next_step: wonton_protocol::RecoveryNextStep::None,
            verification_flow: None,
            key_handoff: None,
        },
        installing_bots: Vec::new(),
        rooms: Vec::new(),
        problem: None,
        push: wonton_protocol::PushStatus::NoEndpoint,
        pending_confirmations: Vec::new(),
        settled_confirmations: Vec::new(),
        background_notifications: Vec::new(),
        bots: bots
            .iter()
            .map(|user_id| BotView {
                chat: Default::default(),
                availability: Default::default(),
                user_id: (*user_id).to_owned(),
                bot: BotRef {
                    plugin_id: "app.hermes".into(),
                    bot: "default".into(),
                },
                web: None,
                web_page: None,
                name: "Hermes".into(),
                avatar: None,
                state_avatars: Vec::new(),
                activity: None,
                saving: false,
                problem: None,
            })
            .collect(),
        own_private_bot: None,
        own_private_bot_problem: None,
    }
}
