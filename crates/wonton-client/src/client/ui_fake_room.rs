//! 假数据入口（`ui_fake.rs`）里的房间和面板网页：一条够长的时间线，和一张带全屏固定浮层的本机网页。
//!
//! 时间线里有几条比屏幕还高的消息，落点在靠前的一条上（进房间停在别处、点“回到底部”再上推，是开发栈上
//! 房间走屏的真实顺序）。面板网页由回环地址上的一个小服务给：公网网页那一类（`TabRef::Url`）不经隧道，
//! 壳直接连它；页面本身能滚，上面盖着一层全屏的固定浮层，右上角一个关闭按钮，照 Navidrome 的手机播放器摆。

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::sync::OnceLock;

use wonton_protocol::{
    BackPagination, MessageContent, PanelItem, PanelSource, RichBlock, RichBlockKind, RichSpan,
    SendState, TabIcon, TabLayout, TabPage, TabRef, TabState, TimelineContent, TimelineDiff,
    TimelineItem, TimelineMessage, TimelineUpdate, TimelineViewport,
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

/// 房间的第一批：全部条目，落点在第三条。之后没有别的批次。
/// 通话中的那个群聊（`ui_fake::CALL_ROOM_ID`）另带顶栏的 widget 入口和“通话中”。
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
        entry_blocked: false,
        key_notice: None,
        confirmations: Vec::new(),
        side_chat: None,
        widgets: if in_call {
            vec![super::ui_fake::call_widget()]
        } else {
            Vec::new()
        },
        call_active: in_call,
    }
}

const ITEMS: usize = 24;
/// 每隔几条来一条比屏幕高的；最后一条也是。
const TALL_EVERY: usize = 4;
const TALL_PARAGRAPHS: usize = 60;
/// 最后一条消息的最后一段：UI 测试按它找真正的底。
const LAST_LINE: &str = "Fixture last line";

fn item(index: usize) -> TimelineItem {
    let own = index % 2 == 1;
    let last = index + 1 == ITEMS;
    let mut paragraphs = if last || index.is_multiple_of(TALL_EVERY) {
        (0..TALL_PARAGRAPHS)
            .map(|line| format!("Fixture message {index}, paragraph {line}: a long reply keeps going for a while."))
            .collect()
    } else {
        vec![format!("Fixture message {index}")]
    };
    if last {
        paragraphs.push(LAST_LINE.into());
    }
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
    TimelineItem {
        id: format!("ui-fake-{index}"),
        content: TimelineContent::Message {
            message: TimelineMessage {
                event_id: Some(format!("$ui-fake-{index}")),
                sender_id: if own {
                    "@owner:home.example"
                } else {
                    "@echo:home.example"
                }
                .into(),
                sender_name: if own { "Owner" } else { "Echo" }.into(),
                is_own: own,
                timestamp_ms: 1_792_000_000_000 + index as u64 * 60_000,
                send_state: SendState::Sent,
                body: MessageContent::Text {
                    body: paragraphs.join("\n\n"),
                    blocks,
                    files: Vec::new(),
                },
                edited: false,
                read_by: Vec::new(),
                reply: None,
                side_chat: None,
                preview: None,
                actions: Vec::new(),
                group: None,
                receipt: None,
                starts_time_group: false,
            },
        },
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

/// 回环地址上给上面那一页的小服务，进程里只起一次，跟着 App 进程结束。
fn fixture_url() -> Option<String> {
    static URL: OnceLock<Option<String>> = OnceLock::new();
    URL.get_or_init(|| {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
        let port = listener.local_addr().ok()?.port();
        std::thread::Builder::new()
            .name("ui-fake-page".into())
            .spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    // 只有这一页：请求读掉开头就回，不看路径。
                    let _ = stream.read(&mut [0; 2048]);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{PAGE}",
                        PAGE.len()
                    );
                }
            })
            .ok()?;
        Some(format!("http://127.0.0.1:{port}/"))
    })
    .clone()
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
