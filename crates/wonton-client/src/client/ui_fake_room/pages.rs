//! 本机夹具网页和回环地址上的小服务：面板里带浮层的那页、整页打开的那页。

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::LazyLock;

use wonton_protocol::{PanelItem, PanelSource, TabIcon, TabLayout, TabPage, TabRef, TabState};

/// 主面板和房间面板里放的那一页。本机端口占不上时没有这一页。
pub(crate) fn panel() -> Vec<PanelItem> {
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
            native: None,
            opens_in_browser: false,
        },
        attention: false,
        source: PanelSource::User,
        movable: true,
    }]
}

/// 整页打开的那张网页：`cover` 为真的那份写了 `viewport-fit=cover`，自己按 `env(safe-area-inset-*)` 让。
/// 本机端口占不上时没有。
pub(crate) fn corner() -> Option<(TabRef, TabPage)> {
    let url = format!("{}corner", fixture_url()?);
    let page = TabPage {
        title: "Corner".into(),
        icon: TabIcon::Plugin { icon: None },
        url: Some(url.clone()),
        layout: TabLayout::default(),
        state: TabState::Ready,
        reload: 0,
        native: None,
        opens_in_browser: false,
    };
    Some((TabRef::Url { url }, page))
}

/// 面板里的那张网页。播放按钮放一段现场拼成 WAV 的两分钟轻方波，页面把播放位置写在 `#audio` 里：
/// 面板收起时页面的声音断没断，界面测试看这个位置还在不在走。
const PAGE: &str = r##"<!doctype html>
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
<div id="player"><p>Fixture player</p><p id="audio">Fixture audio idle</p><p id="path">Fixture path</p><p id="loads">Fixture load</p><button id="same" onclick="history.pushState({}, '', '/same'); history.replaceState({}, '', '/')">Fixture same document</button><a href="#/album" id="fragment">Fixture fragment</a> <a href="/corner-cover">Fixture cover</a></div>
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
function showPath() {
  document.getElementById('path').textContent = 'Fixture path ' + location.pathname + location.hash;
}
addEventListener('hashchange', showPath);
addEventListener('popstate', showPath);
showPath();
const loads = (Number(sessionStorage.getItem('loads')) || 0) + 1;
sessionStorage.setItem('loads', String(loads));
document.getElementById('loads').textContent = 'Fixture load ' + loads;
audio.addEventListener('timeupdate', report);
document.addEventListener('visibilitychange', report);
setInterval(report, 500);
</script>
</body></html>"##;

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
<div id="body"><p id="probe">Fixture probe</p><p id="path">Fixture path</p><a href="/corner-cover">Fixture cover</a>
<button id="same" onclick="history.pushState({}, '', '/same'); history.replaceState({}, '', '/')">Fixture same document</button>
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
  document.getElementById('path').textContent = 'Fixture path ' + location.pathname;
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
    let path = line.split_whitespace().nth(1).unwrap_or("/");
    let body = body(path);
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

#[cfg(test)]
#[path = "pages_tests.rs"]
mod pages_tests;
