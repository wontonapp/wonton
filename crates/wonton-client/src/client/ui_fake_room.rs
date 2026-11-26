//! 假数据入口（`ui_fake.rs`）里的房间和本机网页：一条够长的时间线，一张带全屏固定浮层的面板网页，
//! 和一张右上角有固定按钮、整页打开的网页。
//!
//! 时间线里有几条比屏幕还高的消息，落点在靠前的一条上（进房间停在别处、点“回到底部”再上推，是开发栈上
//! 房间走屏的真实顺序）。面板网页由回环地址上的一个小服务给：公网网页那一类（`TabRef::Url`）不经隧道，
//! 壳直接连它；页面本身能滚，上面盖着一层全屏的固定浮层，右上角一个关闭按钮，照 Navidrome 的手机播放器摆。
//! 整页的那张（[`corner`]）照 Immich、Actual 这类现成应用摆：顶上、底下各一条固定的栏，右上角一个按钮，
//! 出口胶囊压不压得到它由界面测试看（ADR-0095“铺满”）；它声明了 `theme-color`，页面上的按钮能换掉或去掉这个声明，
//! 壳让出来的顶条跟不跟页面的颜色也由界面测试看。

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::LazyLock;

use wonton_protocol::{
    BackPagination, ChoiceSend, ChoicesState, ConfirmationView, ImageCaption, ImageContent,
    MediaHandle, MessageAction, MessageChoice, MessageChoices, MessageContent, MessageGroup,
    MessageStatus, PanelItem, PanelSource, QuotedContent, QuotedMessage, ReplyQuote, RichBlock,
    RichBlockKind, RichLink, RichLinkKind, RichSpan, RoomCommand, SendFailure, SendState,
    SideChatCard, SideChatState, TabIcon, TabLayout, TabPage, TabRef, TabState, TimelineContent,
    TimelineDiff, TimelineItem, TimelineMessage, TimelineUpdate, TimelineViewport,
};

/// 主面板和房间面板里放的那一页。本机端口占不上时没有这一页。
pub(super) fn panel() -> Vec<PanelItem> {
    let Some(url) = fixture_url() else {
        return Vec::new();
    };
    vec![PanelItem {
        tab: TabRef::Url { url: url.clone() },
        page: TabPage {
            title: "Player".into(),
            icon: TabIcon::Plugin { icon: None },
            url: Some(url),
            layout: TabLayout::default(),
            state: TabState::Ready,
            reload: 0,
            opens_in_browser: false,
        },
        attention: false,
        source: PanelSource::User,
        movable: true,
    }]
}

/// 整页打开的那张网页：`cover` 为真的那份写了 `viewport-fit=cover`，自己按 `env(safe-area-inset-*)` 让。
/// 本机端口占不上时没有。
pub(super) fn corner() -> Option<(TabRef, TabPage)> {
    let url = format!("{}corner", fixture_url()?);
    let page = TabPage {
        title: "Corner".into(),
        icon: TabIcon::Plugin { icon: None },
        url: Some(url.clone()),
        layout: TabLayout::default(),
        state: TabState::Ready,
        reload: 0,
        opens_in_browser: false,
    };
    Some((TabRef::Url { url }, page))
}

/// 房间的第一批：全部条目，落点在第三条。之后的批次由 `ui_fake_timeline` 按壳的操作推。
/// 通话中的那个群聊（`ui_fake::CALL_ROOM_ID`）另带顶栏的 widget 入口和“通话中”。
/// Echo 一直在输入，带三条命令（ADR-0109 的命令表和正在输入）。
/// 跨日的地方像核心一样先放一条日期分隔：第一条前和 `DAY_BREAK_INDEX` 前各一条；第一条前那条和它上面的
/// `TimelineStart` 排在一起，和真核心翻到房间开头时的样子一样。
pub(super) fn timeline(room_id: &str) -> TimelineUpdate {
    let in_call = room_id == super::ui_fake::CALL_ROOM_ID;
    let mut items: Vec<TimelineItem> = Vec::with_capacity(ITEMS + 3);
    items.push(timeline_start());
    items.push(divider(TOP_INDEX));
    for index in 0..ITEMS {
        if index == DAY_BREAK_INDEX {
            items.push(divider(index));
        }
        items.push(item(index));
    }
    // 走查进房落点用：`WONTON_UI_FAKE_ANCHOR=last` 让落点就是最新一条，`placeAnchor` 走贴底那一支。
    let anchor = if std::env::var("WONTON_UI_FAKE_ANCHOR").as_deref() == Ok("last") {
        items.last().map(|item| item.id.clone())
    } else {
        Some("ui-fake-2".into())
    };
    TimelineUpdate {
        unread_jump: None,
        anchor,
        diffs: vec![TimelineDiff::Reset { items }],
        back_pagination: BackPagination::ReachedStart,
        viewport: TimelineViewport::Away { new_messages: 0 },
        scroll_to_bottom: false,
        panel: panel(),
        panel_selected: None,
        key_notice: None,
        // Echo 的房间里挂着一张等回答的确认卡片，输入框上方的入口由界面测试看。
        confirmations: if room_id == super::ui_fake::ROOM_ID {
            vec![ConfirmationView {
                card: super::ui_fake_devices::card(),
                sending: None,
                sending_choice_id: None,
                problem: None,
                window: None,
            }]
        } else {
            Vec::new()
        },
        side_chat: None,
        widgets: if in_call {
            vec![super::ui_fake::call_widget()]
        } else {
            Vec::new()
        },
        call_active: in_call,
        // UI 测试的假房间：私聊（顶栏显示 Bot）不带；通话群是群聊，带一个在场的 AI。
        // 假数据的成员名单视为已取全，所以都是 Some。
        room_bots: if in_call {
            Some(vec!["@hermes:home.example".into()])
        } else {
            Some(Vec::new())
        },
        typing: vec!["Echo".into()],
        commands: commands(),
        commands_omitted: 0,
        focus: None,
        jump: wonton_protocol::JumpState::Idle,
        back_to: None,
        editing: None,
        focus_revision: 0,
    }
}

/// 挂在 [`SIDE_ROOT_INDEX`] 那条消息上的旁聊卡片。
pub(super) fn side_card() -> SideChatCard {
    SideChatCard {
        root_event_id: format!("$ui-fake-{SIDE_ROOT_INDEX}"),
        title: "Fixture side chat".into(),
        reply_count: SIDE_ITEMS as u64,
        unread_count: 0,
        state: SideChatState::Open,
    }
}

/// 旁聊里的一批：几条短回复，最底下一条是落点，顶上是这个旁聊的标题和状态。
pub(super) fn side_timeline(room_id: &str) -> TimelineUpdate {
    let items: Vec<TimelineItem> = (0..SIDE_ITEMS)
        .map(|index| {
            let own = index % 2 == 1;
            let message = text_message(
                &format!("side-{index}"),
                if own { OWNER } else { ECHO },
                FIRST_AT_MS + index as u64 * 60_000,
                &[format!("Fixture side {index}")],
            );
            TimelineItem {
                id: format!("side-{index}"),
                content: TimelineContent::Message { message },
            }
        })
        .collect();
    let mut update = timeline(room_id);
    update.anchor = items.last().map(|item| item.id.clone());
    update.viewport = TimelineViewport::Bottom;
    update.diffs = vec![TimelineDiff::Reset { items }];
    update.side_chat = Some(side_card());
    update.typing = Vec::new();
    update
}

/// 刚开的旁聊的一批：只有主人发出去的第一句。假入口没有服务器，根和这条都由 `start_side_chat` 现造。
pub(super) fn started_timeline(room_id: &str, body: &str) -> TimelineUpdate {
    let message = text_message("side-new-0", OWNER, FIRST_AT_MS, &[body.to_owned()]);
    let items = vec![TimelineItem {
        id: "side-new-0".into(),
        content: TimelineContent::Message { message },
    }];
    let mut update = timeline(room_id);
    update.anchor = Some("side-new-0".into());
    update.viewport = TimelineViewport::Bottom;
    update.diffs = vec![TimelineDiff::Reset { items }];
    update.side_chat = Some(side_card());
    update.typing = Vec::new();
    update
}

fn commands() -> Vec<RoomCommand> {
    [
        ("help", "Show help", None),
        ("model", "Switch model", Some("model name")),
        ("new", "Start over", None),
        ("photo album", "Pick from an album", None),
        ("summarize-everything-in-this-thread", "Long summary", None),
    ]
    .into_iter()
    .map(|(name, description, hint)| RoomCommand {
        bot_user_id: ECHO.into(),
        bot_name: "Echo".into(),
        name: name.into(),
        description: description.into(),
        hint: hint.map(Into::into),
        text: format!("/{name}"),
    })
    .collect()
}

pub(super) const ITEMS: usize = 27;
pub(super) const OWNER: &str = "@owner:home.example";
pub(super) const ECHO: &str = "@echo:home.example";
/// 最早一条的时间；下标每大一条晚一分钟（下标大 = 屏幕上更靠下 = 更晚）。
const FIRST_AT_MS: u64 = 1_792_000_000_000;
/// 从这条起（更靠下的那一段）再往后一天：列表里要有一段跨日，日期分隔那一行才有得看。
const DAY_BREAK_INDEX: usize = 11;
const DAY_MS: u64 = 86_400_000;
/// 最顶上那条（下标 0）再往前两天，单独占一天：列表最顶上也要有一条日期分隔。
const TOP_INDEX: usize = 0;
const TOP_DAYS_BACK: u64 = 2 * DAY_MS;

/// 第 `index` 条的发送时间。相邻两条隔一分钟，时间随下标只增不减；第 11 条起是后一天，最顶上的第 0 条再早两天。
pub(super) fn at_ms(index: usize) -> u64 {
    let later = if index >= DAY_BREAK_INDEX { DAY_MS } else { 0 };
    let earliest = if index == TOP_INDEX { TOP_DAYS_BACK } else { 0 };
    FIRST_AT_MS + index as u64 * 60_000 + later - earliest
}

/// 回复第 5 条的那条（自己发的）；它和之前自己的消息 Echo 都读到了，之后的没有。
pub(super) const REPLY_INDEX: usize = 21;
/// Echo 带两个选项的那条。
pub(super) const CHOICES_INDEX: usize = 22;
/// 一条宽表格：比气泡宽，格子上的横向拖动要能滚表格、竖着滑照旧滑时间线。
pub(super) const TABLE_INDEX: usize = 23;
/// 一条“表情 + 一行字”的短消息，下一条长中文回复引用它：引用块要只有名字加一两行高。
pub(super) const QUOTED_INDEX: usize = 19;
/// 引用上一条的长中文回复：几段长短不一，有的末行写满，用来查时间戳和正文重叠、长段被截断。
pub(super) const LONG_REPLY_INDEX: usize = 20;
/// 引用一条不在房间里的消息：跳到它要翻到房间开头，最后给“找不到”。
pub(super) const MISSING_QUOTE_INDEX: usize = 18;
/// 名字很长的发送者：群聊里名字只许占一行。
pub(super) const LONG_NAME_INDEX: usize = 22;
/// 删除的消息：菜单里没有任何动作，只剩发送时间；顺便看读屏念不念“已编辑”。
pub(super) const REDACTED_INDEX: usize = 17;
/// 一条只有 `wonton://` 链接的消息，带查询和结尾斜杠：点它要剥掉后就地打开“其他服务器”。
pub(super) const APP_LINK_INDEX: usize = 6;
/// 认不得的 `wonton:` 链接（单斜杠写法）：点它要提示一句，不开白页。
pub(super) const UNKNOWN_APP_LINK_INDEX: usize = 7;
pub(super) const APP_LINK: &str = "wonton://settings/peers/?from=chat";
pub(super) const UNKNOWN_APP_LINK: &str = "wonton:tab/from-the-future";
/// 自己发的、发不出去的一条：旁边红色感叹号的点击范围要看它。
pub(super) const FAILED_SEND_INDEX: usize = 24;
/// Echo 带一个发送失败的选项：失败原因和重试要看它。
pub(super) const FAILED_CHOICE_INDEX: usize = 25;
/// 失败选项的原因，界面上应当收在“详情”里。
pub(super) const FAILED_CHOICE_REASON: &str = "Server timed out while sending";
/// 这条下面挂着一张旁聊卡，点进去是旁聊页：界面测试用它查顶栏下面的叠放顺序（ADR-0093）。
/// 就挂在进房间的落点那条上，不用翻页；照片条不走 `decorate`，别选照片的下标。
pub(super) const SIDE_CHAT_TOP_INDEX: usize = 2;
// 中文假数据放在数据文件里：它是界面测试的输入，不是界面文字，界面文字检查（ADR-0043）只管源码里的字符串。
const QUOTED_TEXT: &str = include_str!("ui_fake_text/quoted.txt");
const LONG_REPLY: &str = include_str!("ui_fake_text/long_reply.txt");
/// 带旁聊卡片的那条：点了进那个旁聊。
pub(super) const SIDE_ROOT_INDEX: usize = 5;
/// 旁聊里的条数。
const SIDE_ITEMS: usize = 4;
/// 每隔几条来一条比屏幕高的；最后一条也是。
const TALL_EVERY: usize = 4;
const TALL_PARAGRAPHS: usize = 60;
/// 最后一条消息的最后一段：UI 测试按它找真正的底。
const LAST_LINE: &str = "Fixture last line";
/// 第 22 条的发送者名字，长到一行放不下。
const LONG_NAME: &str = "Echo the group bot with a really long account name";
/// 最后一条是自己刚发、还没送达的：待发脚注“已保存，等待送达”和时钟由它验。
const SENDING_INDEX: usize = ITEMS - 1;

/// 一次发送的三张照片（`item()` 里第 5..7 条）：查看器同组翻页的走屏用，字节在 [`media`]。
/// 第 3 条是单张照片（没有组）：查看器不该翻页、也不该显示页码。
const PHOTO_GROUP_START: usize = 5;
const PHOTO_GROUP_COUNT: usize = 3;
const PHOTO_SOLO_INDEX: usize = 3;
const PHOTO_TOKEN: &str = "ui-fake-photo";
const PHOTO_WIDTH: u64 = 320;
const PHOTO_HEIGHT: u64 = 240;
/// 四张照片的颜色，BGR 像素序：深蓝、琥珀、绿、紫（紫是单张那幅）。
const PHOTO_COLORS: [[u8; 3]; 4] = [
    [180, 105, 25],
    [25, 165, 190],
    [90, 160, 70],
    [150, 60, 120],
];

/// 日期分隔：核心在跨日处发的虚拟条目，时间取那一天的第一条消息（`index` 就是它）。
fn divider(index: usize) -> TimelineItem {
    TimelineItem {
        id: format!("ui-fake-day-{index}"),
        content: TimelineContent::DateDivider {
            timestamp_ms: at_ms(index),
        },
    }
}

/// 房间开头：SDK 固定放在最前面的条目，翻到头时它上面是那条分隔。
fn timeline_start() -> TimelineItem {
    TimelineItem {
        id: "ui-fake-start".into(),
        content: TimelineContent::TimelineStart,
    }
}

/// 这一条消息的正文：几类固定样本，其余是一句普通的话或一条长回复，最后一条补上找底用的那句话。
fn paragraphs(index: usize, last: bool) -> Vec<String> {
    let mut paragraphs = if index == CHOICES_INDEX {
        vec![
            "Fixture approval".into(),
            "✅ = approve once\n❌ = deny".into(),
        ]
    } else if index == APP_LINK_INDEX {
        vec![APP_LINK.into()]
    } else if index == UNKNOWN_APP_LINK_INDEX {
        vec![UNKNOWN_APP_LINK.into()]
    } else if index == FAILED_SEND_INDEX {
        vec!["Fixture failed send".into()]
    } else if index == FAILED_CHOICE_INDEX {
        vec![
            "Fixture choices with a failure".into(),
            "⏳ = ask later\n🔁 = retry tomorrow".into(),
        ]
    } else if index == QUOTED_INDEX {
        vec![QUOTED_TEXT.trim_end().into()]
    } else if index == TABLE_INDEX {
        vec!["Fixture table".into()]
    } else if index == LONG_REPLY_INDEX {
        LONG_REPLY
            .trim_end()
            .split("\n\n")
            .map(Into::into)
            .collect()
    } else if last || index.is_multiple_of(TALL_EVERY) {
        (0..TALL_PARAGRAPHS)
            .map(|line| format!("Fixture message {index}, paragraph {line}: a long reply keeps going for a while."))
            .collect()
    } else {
        vec![format!("Fixture message {index}")]
    };
    if last {
        paragraphs.push(LAST_LINE.into());
    }
    paragraphs
}

/// 状态、回复、选项、旁聊卡片和两处失败标记：`item` 里只留正文。
fn decorate(index: usize, own: bool, message: &mut TimelineMessage) {
    let status = if index <= REPLY_INDEX {
        MessageStatus::Read
    } else {
        MessageStatus::Sent
    };
    message.status =
        (own && index != SENDING_INDEX && index != FAILED_SEND_INDEX).then_some(status);
    if index == SENDING_INDEX {
        message.send_state = SendState::Sending { progress: None };
    }
    if let Some(reply) = quote(index) {
        message.reply = Some(reply);
    }
    if index == MISSING_QUOTE_INDEX {
        message.reply = Some(ReplyQuote::Unavailable {
            event_id: "$ui-fake-missing".into(),
        });
    }
    if index == CHOICES_INDEX {
        message.choices = Some(choices());
    }
    if index == SIDE_CHAT_TOP_INDEX {
        message.side_chat = Some(SideChatCard {
            root_event_id: message.event_id.clone().unwrap_or_default(),
            title: "Fixture side chat".into(),
            reply_count: 1,
            unread_count: 0,
            state: SideChatState::Open,
        });
    }
    if index == FAILED_SEND_INDEX {
        message.send_state = SendState::Failed {
            failure: SendFailure::Upstream,
            details: Some(FAILED_CHOICE_REASON.into()),
            recoverable: true,
        };
    }
    if index == FAILED_CHOICE_INDEX {
        message.choices = Some(MessageChoices {
            options: [("⏳", "ask later"), ("🔁", "retry tomorrow")]
                .into_iter()
                .map(|(key, label)| MessageChoice {
                    key: key.into(),
                    label: Some(label.into()),
                    chosen: false,
                    send: if key == "⏳" {
                        ChoiceSend::Failed {
                            message: FAILED_CHOICE_REASON.into(),
                        }
                    } else {
                        ChoiceSend::Idle
                    },
                })
                .collect(),
            state: ChoicesState::Open,
        });
    }
}

/// 带链接的两条：正文就是那个 `wonton:` 地址。
fn link_url(index: usize) -> Option<&'static str> {
    match index {
        APP_LINK_INDEX => Some(APP_LINK),
        UNKNOWN_APP_LINK_INDEX => Some(UNKNOWN_APP_LINK),
        _ => None,
    }
}

/// 把正文换成一行链接：这两条是照片消息，链接得挂在照片的说明上，所以要等挑完条目再说。
fn set_link(item: &mut TimelineItem, url: &str) {
    let TimelineContent::Message { message } = &mut item.content else {
        return;
    };
    let block = RichBlock {
        kind: RichBlockKind::Paragraph,
        depth: 0,
        quoted: false,
        spans: vec![RichSpan {
            text: url.into(),
            link: Some(RichLink {
                url: url.into(),
                kind: RichLinkKind::Other,
            }),
            ..RichSpan::default()
        }],
    };
    match &mut message.body {
        MessageContent::Text { body, blocks, .. } => {
            *body = url.into();
            blocks.clear();
            blocks.push(block);
        }
        MessageContent::Image { image } => {
            image.caption = Some(ImageCaption {
                body: url.into(),
                blocks: vec![block],
            });
        }
        _ => {}
    }
}

fn item(index: usize) -> TimelineItem {
    if let Some(mut photo) = photo(index) {
        if let Some(url) = link_url(index) {
            set_link(&mut photo, url);
        }
        return photo;
    }
    // 8..=10 都算 Echo 的，凑一组连发的消息给并排的气泡做样子，第 24 条是自己发不出去的一条，最后一条是自己还没送达的，
    // 其余一左一右轮流。
    let own = (index % 2 == 1 && !(8..=10).contains(&index))
        || index == FAILED_SEND_INDEX
        || index == SENDING_INDEX;
    let last = index + 1 == ITEMS;
    let paragraphs = paragraphs(index, last);
    let mut message = text_message(
        &format!("ui-fake-{index}"),
        if own { OWNER } else { ECHO },
        at_ms(index),
        &paragraphs,
    );
    if index == TABLE_INDEX
        && let MessageContent::Text { blocks, .. } = &mut message.body
    {
        blocks.push(table());
    }
    if index == LONG_NAME_INDEX {
        message.sender_name = LONG_NAME.into();
    }
    if index == REDACTED_INDEX {
        message.body = MessageContent::Redacted;
        message.actions.clear();
        message.edited = true;
    }
    decorate(index, own, &mut message);
    let mut item = TimelineItem {
        id: format!("ui-fake-{index}"),
        content: TimelineContent::Message { message },
    };
    if let Some(url) = link_url(index) {
        set_link(&mut item, url);
    }
    item
}

/// 引用条：`REPLY_INDEX` 引用第 5 条，`LONG_REPLY_INDEX` 引用那条短消息，其余的没有。
fn quote(index: usize) -> Option<ReplyQuote> {
    let (quoted, text): (usize, &str) = match index {
        REPLY_INDEX => (SIDE_ROOT_INDEX, "Fixture message 5"),
        LONG_REPLY_INDEX => (QUOTED_INDEX, QUOTED_TEXT.trim_end()),
        _ => return None,
    };
    Some(ReplyQuote::Ready {
        message: QuotedMessage {
            event_id: format!("$ui-fake-{quoted}"),
            sender_id: OWNER.into(),
            sender_name: "Owner".into(),
            content: QuotedContent::Text { text: text.into() },
        },
    })
}

/// `TABLE_INDEX` 那条上挂的宽表格：比气泡宽，格子上的横向拖动要能滚表格。
fn table() -> RichBlock {
    RichBlock {
        kind: RichBlockKind::Table {
            rows: (0..4)
                .map(|row| {
                    (0..5)
                        .map(|cell| format!("Fixture cell {row}-{cell} wide"))
                        .collect()
                })
                .collect(),
        },
        depth: 0,
        quoted: false,
        spans: Vec::new(),
    }
}

/// `CHOICES_INDEX` 那两个选项，等界面来点。
fn choices() -> MessageChoices {
    MessageChoices {
        options: [("✅", "approve once"), ("❌", "deny")]
            .into_iter()
            .map(|(key, label)| MessageChoice {
                key: key.into(),
                label: Some(label.into()),
                chosen: false,
                send: ChoiceSend::Idle,
            })
            .collect(),
        state: ChoicesState::Open,
    }
}

/// 第 5–7 条是一次发送的三张照片，第 3 条是没组的单张；其余下标不是照片。
fn photo(index: usize) -> Option<TimelineItem> {
    if (PHOTO_GROUP_START..PHOTO_GROUP_START + PHOTO_GROUP_COUNT).contains(&index) {
        return Some(photo_item(
            index,
            index - PHOTO_GROUP_START,
            Some(PHOTO_GROUP_COUNT as u32),
        ));
    }
    (index == PHOTO_SOLO_INDEX).then(|| photo_item(index, PHOTO_GROUP_COUNT, None))
}

/// 一条发出去了的文字消息，段落之间空一行；菜单里有复制和回复（左滑回复要有回复）。组内位置和勾由调用方定。
pub(super) fn text_message(
    id: &str,
    sender: &str,
    timestamp_ms: u64,
    paragraphs: &[String],
) -> TimelineMessage {
    let own = sender == OWNER;
    let blocks = paragraphs
        .iter()
        .map(|text| RichBlock {
            kind: RichBlockKind::Paragraph,
            depth: 0,
            quoted: false,
            spans: vec![RichSpan {
                text: text.clone(),
                ..RichSpan::default()
            }],
        })
        .collect();
    let body = paragraphs.join("\n\n");
    let event_id = format!("${id}");
    let sender_name = if own { "Owner" } else { "Echo" };
    let mut actions = vec![
        MessageAction::Copy { text: body.clone() },
        MessageAction::Reply {
            message: QuotedMessage {
                event_id: event_id.clone(),
                sender_id: sender.into(),
                sender_name: sender_name.into(),
                content: QuotedContent::Text { text: body.clone() },
            },
        },
    ];
    // ADR-0109 的菜单顺序是回复、复制、编辑、转发：前两条造的时候换了位，这里换回来。
    actions.swap(0, 1);
    // 编辑的规则照真核心的菜单：只有自己的文字消息给（`chat/menu.rs`），不看正文空不空。
    if own {
        actions.push(MessageAction::Edit { text: body.clone() });
    }
    actions.push(MessageAction::Forward);
    // 撤回照真核心固定放最后：自己的消息才有。
    if own {
        actions.push(MessageAction::Redact);
    }
    TimelineMessage {
        mentions_me: false,
        unread_anchor: false,
        actions,
        event_id: Some(event_id),
        sender_id: sender.into(),
        sender_name: sender_name.into(),
        is_own: own,
        timestamp_ms,
        send_state: SendState::Sent,
        body: MessageContent::Text {
            body,
            blocks,
            files: Vec::new(),
        },
        edited: false,
        read_by: Vec::new(),
        reply: None,
        side_chat: None,
        preview: None,
        group: None,
        receipt: None,
        starts_time_group: false,
        run: wonton_protocol::MessageRun::Alone,
        status: None,
        choices: None,
        sender_avatar: None,
    }
}

fn photo_item(index: usize, position: usize, count: Option<u32>) -> TimelineItem {
    let handle = MediaHandle {
        token: format!("{PHOTO_TOKEN}-{position}"),
    };
    // 说明文字就是原来这条的文字消息：现有走查按 “Fixture message N” 找这一条，
    // 长按菜单（回复、转发）和引用也靠它，换成照片后这些还得照旧。6、7 说明会被 `set_link` 换成链接地址。
    let own = index != PHOTO_SOLO_INDEX;
    let text = format!("Fixture message {index}");
    let blocks = vec![RichBlock {
        kind: RichBlockKind::Paragraph,
        depth: 0,
        quoted: false,
        spans: vec![RichSpan {
            text: text.clone(),
            ..RichSpan::default()
        }],
    }];
    let event_id = format!("$ui-fake-{index}");
    let sender_id = if own { OWNER } else { ECHO };
    let sender_name = if own { "Owner" } else { "Echo" };
    TimelineItem {
        id: format!("ui-fake-{index}"),
        content: TimelineContent::Message {
            message: TimelineMessage {
                mentions_me: false,
                unread_anchor: false,
                event_id: Some(event_id.clone()),
                sender_id: sender_id.into(),
                sender_name: sender_name.into(),
                is_own: own,
                timestamp_ms: 1_792_000_000_000 + index as u64 * 60_000,
                send_state: SendState::Sent,
                body: MessageContent::Image {
                    image: ImageContent {
                        thumbnail: Some(handle.clone()),
                        media: handle,
                        filename: format!("photo-{position}.bmp"),
                        mime_type: Some("image/bmp".into()),
                        size: None,
                        width: Some(PHOTO_WIDTH),
                        height: Some(PHOTO_HEIGHT),
                        caption: Some(ImageCaption {
                            body: text.clone(),
                            blocks,
                        }),
                    },
                },
                edited: false,
                read_by: Vec::new(),
                reply: None,
                // 第 5 条也是旁聊的根：旁聊卡片挂在它的照片下面（ADR-0093 的走查）。
                side_chat: (index == SIDE_ROOT_INDEX).then(side_card),
                preview: None,
                actions: {
                    let mut actions = vec![
                        MessageAction::Copy { text: text.clone() },
                        MessageAction::Reply {
                            message: QuotedMessage {
                                event_id,
                                sender_id: sender_id.into(),
                                sender_name: sender_name.into(),
                                content: QuotedContent::Text { text },
                            },
                        },
                        MessageAction::Forward,
                    ];
                    actions.swap(0, 1);
                    actions
                },
                group: count.map(|count| MessageGroup {
                    id: "000102030405060708090a0b0c0d0e0f".into(),
                    index: position as u32,
                    count,
                }),
                receipt: None,
                starts_time_group: false,
                run: wonton_protocol::MessageRun::Alone,
                status: None,
                choices: None,
                sender_avatar: None,
            },
        },
    }
}

/// 假房间图片的原文件：一张无压缩的 24 位 BMP，四张颜色不同（最后一张是单张那幅）。
/// 假数据入口的 [`ClientCore::load_media`] 直接在这里取，不进下载。
pub(super) fn media(token: &str) -> Option<Vec<u8>> {
    let position: usize = token
        .strip_prefix(PHOTO_TOKEN)?
        .strip_prefix('-')?
        .parse()
        .ok()?;
    PHOTO_COLORS
        .get(position)
        .map(|&bgr| bitmap(PHOTO_WIDTH as u32, PHOTO_HEIGHT as u32, bgr))
}

/// 无压缩的 24 位 BMP：头 + 从下往上的 BGR 像素行，每行补齐到 4 字节。
fn bitmap(width: u32, height: u32, bgr: [u8; 3]) -> Vec<u8> {
    let row = width as usize * 3;
    let stride = row.div_ceil(4) * 4;
    let size = 54 + stride * height as usize;
    let mut out = vec![0u8; size];
    out[0..2].copy_from_slice(b"BM");
    out[2..6].copy_from_slice(&(size as u32).to_le_bytes());
    out[10..14].copy_from_slice(&54u32.to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&width.to_le_bytes());
    out[22..26].copy_from_slice(&height.to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..height as usize {
        let base = 54 + y * stride;
        for x in 0..width as usize {
            out[base + x * 3..base + x * 3 + 3].copy_from_slice(&bgr);
        }
    }
    out
}

/// 面板里的那张网页。播放按钮放一段现场拼成 WAV 的两分钟轻方波，页面把播放位置写在 `#audio` 里：
/// 面板收起时页面的声音断没断，界面测试看这个位置还在不在走。
const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Player</title>
<style>
html, body { margin: 0; background: #101418; color: #fff; font: 17px system-ui; }
#library { height: 300vh; }
#player { position: fixed; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; background: #1b2733; }
#close { position: fixed; top: 78px; right: 8px; width: 60px; height: 60px; font-size: 28px; }
#play { position: fixed; top: 160px; left: 120px; width: 120px; height: 60px; }
</style></head>
<body>
<div id="library">Fixture library</div>
<div id="player"><p>Fixture player</p><p id="audio">Fixture audio idle</p></div>
<button id="close" aria-label="Fixture close" onclick="this.setAttribute('aria-label', 'Fixture closed')">×</button>
<button id="play" aria-label="Fixture play" onclick="play()">▶</button>
<script>
const audio = new Audio();
let pauses = 0;
function tone(seconds) {
  const rate = 8000, count = rate * seconds, view = new DataView(new ArrayBuffer(44 + count));
  const text = (at, value) => { for (let i = 0; i < value.length; i++) view.setUint8(at + i, value.charCodeAt(i)); };
  text(0, 'RIFF'); view.setUint32(4, 36 + count, true); text(8, 'WAVEfmt ');
  view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, rate, true); view.setUint32(28, rate, true);
  view.setUint16(32, 1, true); view.setUint16(34, 8, true);
  text(36, 'data'); view.setUint32(40, count, true);
  for (let i = 0; i < count; i++) view.setUint8(44 + i, i % 40 < 20 ? 129 : 127);
  return URL.createObjectURL(new Blob([view.buffer], { type: 'audio/wav' }));
}
function report() {
  document.getElementById('audio').textContent =
    'Fixture audio ' + (audio.paused ? 'paused ' : 'playing ') + Math.floor(audio.currentTime) + ' pauses ' + pauses;
}
function play() { audio.src = tone(120); audio.play().then(report, report); }
audio.addEventListener('pause', () => { pauses += 1; report(); });
audio.addEventListener('timeupdate', report);
document.addEventListener('visibilitychange', report);
setInterval(report, 500);
</script>
</body></html>"#;

/// 整页的那张网页。`{viewport}` 换成有没有 `viewport-fit=cover`；`#probe` 写着是哪一份、页面自己量到的安全区和视口高度，
/// 界面测试失败时带进报错里。页面比屏幕长，滚下去以后固定的栏还在原位。
/// `recolor` 换掉声明的 `theme-color` 和顶栏的颜色；给 `null` 就去掉声明，顶栏改成和 `body` 的背景一样。
/// `mismatch` 只把声明的 `theme-color` 换成和顶栏不同的浅色（Navidrome 就是这样）；`veil` 给顶栏盖一层半透明的。
const CORNER: &str = r##"<!doctype html>
<html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1{viewport}">
<title>Corner</title>
<meta name="theme-color" content="#2a6f97">
<style>
html, body { margin: 0; background: #17324d; color: #fff; font: 17px system-ui; }
#bar { position: fixed; top: 0; left: 0; right: 0; height: 48px; padding-top: env(safe-area-inset-top); background: #2a6f97; }
#menu { position: fixed; top: env(safe-area-inset-top); right: 0; width: 60px; height: 48px; font-size: 24px; }
#foot { position: fixed; bottom: 0; left: 0; right: 0; height: 48px; background: #2a6f97; }
#body { padding: 120px 16px; height: 300vh; }
#body button { display: block; margin: 16px 0; font: inherit; }
</style></head>
<body>
<div id="bar">Fixture bar</div>
<button id="menu" aria-label="Fixture corner" onclick="this.setAttribute('aria-label', 'Fixture corner tapped')">≡</button>
<div id="body"><p id="probe">Fixture probe</p><a href="/corner-cover">Fixture cover</a>
<button onclick="recolor('#9d4edd')">Fixture recolor</button>
<button onclick="recolor(null)">Fixture unthemed</button>
<button onclick="mismatch()">Fixture mismatch</button>
<button onclick="veil()">Fixture veil</button></div>
<div id="foot">Fixture foot</div>
<script>
function recolor(color) {
  for (const meta of document.querySelectorAll('meta[name=theme-color]')) {
    if (color) { meta.content = color; } else { meta.remove(); }
  }
  document.getElementById('bar').style.background = color || '#17324d';
}
function mismatch() {
  for (const meta of document.querySelectorAll('meta[name=theme-color]')) meta.content = '#90caf9';
}
function veil() {
  const veil = document.createElement('div');
  veil.style.cssText = 'position:fixed;top:0;left:0;right:0;height:48px;background:rgba(255,255,255,0.5)';
  document.body.appendChild(veil);
}
function probe() {
  const box = document.createElement('div');
  box.style.cssText = 'position:fixed;top:env(safe-area-inset-top);bottom:env(safe-area-inset-bottom);visibility:hidden';
  document.body.appendChild(box);
  const style = getComputedStyle(box), menu = document.getElementById('menu').getBoundingClientRect();
  const fit = document.querySelector('meta[name=viewport]').content.includes('cover') ? 'cover' : 'plain';
  document.getElementById('probe').textContent = 'Fixture probe ' + fit + ' safe ' + style.top + ' ' + style.bottom +
    ' inner ' + innerHeight + ' client ' + document.documentElement.clientHeight + ' menu ' + Math.round(menu.top);
  box.remove();
}
probe();
addEventListener('resize', probe);
setInterval(probe, 500);
</script>
</body></html>"##;

fn body(path: &str) -> String {
    match path {
        "/corner" => CORNER.replace("{viewport}", ""),
        "/corner-cover" => CORNER.replace("{viewport}", ", viewport-fit=cover"),
        _ => PAGE.to_owned(),
    }
}

/// 回环地址上给上面几页的小服务，进程里只起一次，跟着 App 进程结束。
fn fixture_url() -> Option<String> {
    static URL: LazyLock<Option<String>> = LazyLock::new(|| {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
        let port = listener.local_addr().ok()?.port();
        std::thread::Builder::new()
            .name("ui-fake-page".into())
            .spawn(move || {
                // 每个连接单开一条线程：WebView 会先连一条不发数据的预备连接，串行处理会让后面的
                // 正式请求一直等着，页面关掉重开、WebView 重建时页面就会停在骨架。
                for stream in listener.incoming().flatten() {
                    std::thread::spawn(move || serve(stream));
                }
            })
            .ok()?;
        Some(format!("http://127.0.0.1:{port}/"))
    });
    URL.clone()
}

/// 只看请求行里的路径，别的不读；应答完就关连接。
fn serve(mut stream: TcpStream) {
    let mut request = [0; 2048];
    let read = stream.read(&mut request).unwrap_or(0);
    let line = String::from_utf8_lossy(&request[..read]);
    let body = body(line.split_whitespace().nth(1).unwrap_or("/"));
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_page_is_served_and_placed_in_the_panel() {
        let panel = panel();
        let Some(url) = panel.first().and_then(|item| item.page.url.clone()) else {
            panic!("the loopback fixture page did not start");
        };
        let address = url.trim_start_matches("http://").trim_end_matches('/');
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nhost: fixture\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("Fixture close"));
    }

    #[test]
    fn fixture_page_is_served_while_an_idle_connection_is_open() {
        let panel = panel();
        let Some(url) = panel.first().and_then(|item| item.page.url.clone()) else {
            panic!("the loopback fixture page did not start");
        };
        let address = url.trim_start_matches("http://").trim_end_matches('/');
        // WebView 会先连一条不发请求的连接，后面的正式请求不能等它。
        let _idle = std::net::TcpStream::connect(address).unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nhost: fixture\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn corner_page_is_served_with_and_without_viewport_fit() {
        let (tab, page) = corner().expect("the loopback fixture page did not start");
        let url = page.url.clone().unwrap();
        assert_eq!(tab, TabRef::Url { url: url.clone() });
        let (address, _) = url.trim_start_matches("http://").split_once('/').unwrap();
        let get = |path: &str| {
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            // 一次写完：服务只读一次就回，分几次写时没读走的那截会让连接被重置。
            stream
                .write_all(format!("GET {path} HTTP/1.1\r\nhost: fixture\r\n\r\n").as_bytes())
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        let plain = get("/corner");
        assert!(plain.contains("Fixture corner") && !plain.contains("viewport-fit=cover"));
        assert!(get("/corner-cover").contains("viewport-fit=cover"));
    }

    #[test]
    fn photos_come_as_one_group_with_loadable_bytes() {
        let update = timeline("!ui-fake:home.example");
        let [TimelineDiff::Reset { items }] = update.diffs.as_slice() else {
            panic!("the first batch must be a reset");
        };
        let images: Vec<&TimelineMessage> = items
            .iter()
            .filter_map(|item| match &item.content {
                TimelineContent::Message { message }
                    if matches!(message.body, MessageContent::Image { .. }) =>
                {
                    Some(message)
                }
                _ => None,
            })
            .collect();
        // 组里三张（同组、按 index 排好）加一张没组的单张。
        let grouped: Vec<&TimelineMessage> = images
            .iter()
            .copied()
            .filter(|m| m.group.is_some())
            .collect();
        assert_eq!(grouped.len(), PHOTO_GROUP_COUNT);
        for (position, message) in grouped.iter().enumerate() {
            let group = message
                .group
                .as_ref()
                .expect("a grouped photo carries its group");
            assert_eq!(
                (group.index, group.count),
                (position as u32, PHOTO_GROUP_COUNT as u32)
            );
            let MessageContent::Image { image } = &message.body else {
                panic!("a photo message must carry an image");
            };
            assert!(
                media(&image.media.token).is_some(),
                "{} must load",
                image.media.token
            );
            assert_eq!(image.thumbnail, Some(image.media.clone()));
        }
        let solo = images
            .iter()
            .find(|m| m.group.is_none())
            .expect("one photo is ungrouped");
        let MessageContent::Image { image } = &solo.body else {
            panic!("a photo message must carry an image");
        };
        assert!(
            media(&image.media.token).is_some(),
            "{} must load",
            image.media.token
        );
    }

    #[test]
    fn link_messages_show_the_url_on_their_own_line() {
        for (index, url) in [
            (APP_LINK_INDEX, APP_LINK),
            (UNKNOWN_APP_LINK_INDEX, UNKNOWN_APP_LINK),
        ] {
            let item = item(index);
            let TimelineContent::Message { message } = item.content else {
                panic!("item {index} must be a message");
            };
            // 6、7 是照片：链接挂在说明上，不能落在挑照片之前那条被丢掉的文字消息上。
            let MessageContent::Image { image } = message.body else {
                panic!("item {index} must be a photo");
            };
            let caption = image.caption.expect("the link must be the caption");
            assert_eq!(caption.body, url);
            assert_eq!(caption.blocks[0].spans[0].text, url);
            let link = caption.blocks[0].spans[0]
                .link
                .as_ref()
                .expect("the span carries the link");
            assert_eq!(
                (link.url.as_str(), link.kind.clone()),
                (url, RichLinkKind::Other)
            );
        }
    }

    #[test]
    fn timeline_lands_away_from_a_tall_last_message() {
        let update = timeline("!ui-fake:home.example");
        let [TimelineDiff::Reset { items }] = update.diffs.as_slice() else {
            panic!("the first batch must be a reset");
        };
        assert_eq!(items.len(), ITEMS + 3);
        assert_ne!(
            update.anchor.as_deref(),
            items.last().map(|item| item.id.as_str())
        );
        let Some(TimelineContent::Message { message }) = items.last().map(|item| &item.content)
        else {
            panic!("the last item must be a message");
        };
        let MessageContent::Text { blocks, .. } = &message.body else {
            panic!("the last item must be text");
        };
        assert_eq!(blocks.last().unwrap().spans[0].text, LAST_LINE);
        // 最后一条是待发消息：界面测试靠它验“已保存，等待送达”的脚注和时钟。
        assert_eq!(message.send_state, SendState::Sending { progress: None });
        assert_eq!(message.status, None);
    }
}
