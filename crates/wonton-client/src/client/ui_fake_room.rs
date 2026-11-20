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
    BackPagination, ChoiceSend, ChoicesState, MessageAction, MessageChoice, MessageChoices,
    MessageContent, MessageStatus, PanelItem, PanelSource, QuotedContent, QuotedMessage,
    ReplyQuote, RichBlock, RichBlockKind, RichSpan, RoomCommand, SendState, TabIcon, TabLayout,
    TabPage, TabRef, TabState, TimelineContent, TimelineDiff, TimelineItem, TimelineMessage,
    TimelineUpdate, TimelineViewport,
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
pub(super) fn timeline(room_id: &str) -> TimelineUpdate {
    let in_call = room_id == super::ui_fake::CALL_ROOM_ID;
    let items: Vec<TimelineItem> = (0..ITEMS).map(item).collect();
    TimelineUpdate {
        anchor: Some(items[2].id.clone()),
        diffs: vec![TimelineDiff::Reset { items }],
        back_pagination: BackPagination::ReachedStart,
        viewport: TimelineViewport::Away { new_messages: 0 },
        scroll_to_bottom: false,
        panel: panel(),
        panel_selected: None,
        key_notice: None,
        confirmations: Vec::new(),
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
    }
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

pub(super) const ITEMS: usize = 24;
pub(super) const OWNER: &str = "@owner:home.example";
const ECHO: &str = "@echo:home.example";
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
/// 一条“表情 + 一行字”的短消息，下一条长中文回复引用它：引用块要只有名字加一两行高。
pub(super) const QUOTED_INDEX: usize = 19;
/// 引用上一条的长中文回复：几段长短不一，有的末行写满，用来查时间戳和正文重叠、长段被截断。
pub(super) const LONG_REPLY_INDEX: usize = 20;
// 中文假数据放在数据文件里：它是界面测试的输入，不是界面文字，界面文字检查（ADR-0043）只管源码里的字符串。
const QUOTED_TEXT: &str = include_str!("ui_fake_text/quoted.txt");
const LONG_REPLY: &str = include_str!("ui_fake_text/long_reply.txt");
/// 每隔几条来一条比屏幕高的；最后一条也是。
const TALL_EVERY: usize = 4;
const TALL_PARAGRAPHS: usize = 60;
/// 最后一条消息的最后一段：UI 测试按它找真正的底。
const LAST_LINE: &str = "Fixture last line";

fn item(index: usize) -> TimelineItem {
    // 8..=10 都算 Echo 的，凑一组连发的消息给并排的气泡做样子，其余一左一右轮流。
    let own = index % 2 == 1 && !(8..=10).contains(&index);
    let last = index + 1 == ITEMS;
    let mut paragraphs = if index == CHOICES_INDEX {
        vec![
            "Fixture approval".into(),
            "✅ = approve once\n❌ = deny".into(),
        ]
    } else if index == QUOTED_INDEX {
        vec![QUOTED_TEXT.trim_end().into()]
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
    let mut message = text_message(
        &format!("ui-fake-{index}"),
        if own { OWNER } else { ECHO },
        at_ms(index),
        &paragraphs,
    );
    let status = if index <= REPLY_INDEX {
        MessageStatus::Read
    } else {
        MessageStatus::Sent
    };
    message.status = own.then_some(status);
    if index == REPLY_INDEX {
        message.reply = Some(ReplyQuote::Ready {
            message: QuotedMessage {
                event_id: "$ui-fake-5".into(),
                sender_id: OWNER.into(),
                sender_name: "Owner".into(),
                content: QuotedContent::Text {
                    text: "Fixture message 5".into(),
                },
            },
        });
    }
    if index == LONG_REPLY_INDEX {
        message.reply = Some(ReplyQuote::Ready {
            message: QuotedMessage {
                event_id: format!("$ui-fake-{QUOTED_INDEX}"),
                sender_id: OWNER.into(),
                sender_name: "Owner".into(),
                content: QuotedContent::Text {
                    text: QUOTED_TEXT.trim_end().into(),
                },
            },
        });
    }
    if index == CHOICES_INDEX {
        message.choices = Some(MessageChoices {
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
        });
    }
    TimelineItem {
        id: format!("ui-fake-{index}"),
        content: TimelineContent::Message { message },
    }
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
    TimelineMessage {
        actions: vec![
            MessageAction::Copy { text: body.clone() },
            MessageAction::Reply {
                message: QuotedMessage {
                    event_id: event_id.clone(),
                    sender_id: sender.into(),
                    sender_name: sender_name.into(),
                    content: QuotedContent::Text { text: body.clone() },
                },
            },
            MessageAction::Forward,
        ],
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
    fn timeline_lands_away_from_a_tall_last_message() {
        let update = timeline("!ui-fake:home.example");
        let [TimelineDiff::Reset { items }] = update.diffs.as_slice() else {
            panic!("the first batch must be a reset");
        };
        assert_eq!(items.len(), ITEMS);
        assert_ne!(
            update.anchor.as_deref(),
            items.last().map(|item| item.id.as_str())
        );
        let TimelineContent::Message { message } = &items[ITEMS - 1].content else {
            panic!("the last item must be a message");
        };
        let MessageContent::Text { blocks, .. } = &message.body else {
            panic!("the last item must be text");
        };
        assert_eq!(blocks.last().unwrap().spans[0].text, LAST_LINE);
    }
}
