//! 本机夹具网页和回环地址上的小服务：整页打开的那页。

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::LazyLock;

use wonton_protocol::{TabIcon, TabLayout, TabPage, TabRef, TabState};

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
        "/corner-cover" => CORNER.replace("{viewport}", ", viewport-fit=cover"),
        _ => CORNER.replace("{viewport}", ""),
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
