use super::*;
use wonton_matrix::{Message, MessageBody, OwnedUserId, SendStatus};
use wonton_protocol::{MessageContent, RichLinkKind, TimelineContent};

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
    let owner: OwnedUserId = "@owner:wonton.localhost".parse().unwrap();
    let (_, answers) = watch::channel(Default::default());
    let cards = Cards::new(&owner, answers).view();
    let read = ReadMark::default();
    let entries: Vector<TimelineEntry> = vec![text_entry(
        "1",
        "打开 http://app-grafana.home.example:18480/",
    )]
    .into_iter()
    .collect();

    let before = relinked(
        &entries,
        &cards,
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
        &cards,
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
