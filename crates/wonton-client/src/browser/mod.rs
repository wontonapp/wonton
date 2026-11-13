//! 要跨源隔离的插件网页在 Android 上交给系统浏览器打开（ADR-0033 的“跨源隔离”）。
//!
//! Android WebView 不给跨源隔离（COOP/COEP 不生效，也没有 Document-Isolation-Policy），这样的页面在 App 里只会
//! 报错。核心在 [`TabPage::opens_in_browser`](wonton_protocol::TabPage) 里告诉壳不要加载，壳调
//! [`BrowserLink::open`] 换一个浏览器打开的地址：`http://<标签>.localhost:<端口>/…`，由回环上的
//! [`BrowserFront`] 转进隧道。标签和端口都由设备密钥推出来，每台服务器、每个插件固定：浏览器那边的 origin
//! 不变，插件存在浏览器里的数据（登录状态、本地账本）就一直在。

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};

use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::runtime::Handle;
use tokio::sync::watch;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    BotWebView, ChatViewModel, CoreError, HomeViewModel, InstalledPlugin, PluginsViewModel,
};
use wonton_tunnel::ProxyCredential;
use wonton_tunnel::browser_front::{
    BrowserFront, BrowserRefusal, BrowserRoute, BrowserRoutes, LABEL_LEN,
};

use crate::l10n::{CATALOG, language};

/// 壳用来显示插件网页的引擎。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebEngine {
    /// Android System WebView：没有跨源隔离。
    AndroidWebView,
    /// iOS 的 WKWebView：看 COOP/COEP 响应头就给跨源隔离。
    WebKit,
}

pub(crate) const ENGINE: WebEngine = if cfg!(target_os = "android") {
    WebEngine::AndroidWebView
} else {
    WebEngine::WebKit
};

/// 这个插件的网页要不要交给系统浏览器：清单声明了跨源隔离，而这个引擎给不了。
pub(crate) fn opens_in_browser(plugin: &InstalledPlugin, engine: WebEngine) -> bool {
    plugin.web_url.is_some() && isolated_page(plugin.cross_origin_isolated, engine)
}

/// Bot 的网页要不要交给系统浏览器：同上，隔离与否由 Bot 页面自己声明。
pub(crate) fn opens_in_browser_bot(isolated: bool, engine: WebEngine) -> bool {
    isolated_page(isolated, engine)
}

fn isolated_page(isolated: bool, engine: WebEngine) -> bool {
    engine == WebEngine::AndroidWebView && isolated
}

/// 浏览器入口的首选端口落在这个范围：避开特权端口和 Android 的临时端口（32768 起）。
const PORT_BASE: u16 = 10_000;
const PORT_SPAN: u16 = 20_000;

pub(crate) struct BrowserLink {
    runtime: Handle,
    routes: Arc<Routes>,
    front: Mutex<Option<Arc<BrowserFront>>>,
}

impl BrowserLink {
    /// `device_key` 是设备密钥：标签和端口从它推出来，换设备身份（重新配对）后跟着换。
    pub(crate) fn new(
        runtime: Handle,
        device_key: [u8; 32],
        home: watch::Receiver<HomeViewModel>,
        plugins: watch::Receiver<PluginsViewModel>,
        chat: watch::Receiver<ChatViewModel>,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            routes: Arc::new(Routes {
                device_key,
                home,
                plugins,
                chat,
            }),
            front: Mutex::new(None),
        })
    }

    /// 插件网页地址 `url`（`TabPage.url`）在浏览器里打开用的地址。只收要跨源隔离的插件的网页。
    pub(crate) fn open(&self, url: &str) -> Result<String, CoreError> {
        let not_isolated = || CoreError::InvalidInput {
            reason: l10n!(CATALOG, language(), "browser-page-not-isolated"),
        };
        let page = Url::parse(url).map_err(|_| not_isolated())?;
        let page_id = self
            .routes
            .isolated_pages()
            .into_iter()
            .find(|(_, origin)| *origin == page.origin().ascii_serialization())
            .map(|(id, _)| id)
            .ok_or_else(not_isolated)?;
        let server = self.routes.server().ok_or_else(|| CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "browser-not-paired"),
        })?;
        let front = self.front()?;
        let label = label(&self.routes.device_key, &server, &page_id);
        let mut target = format!("{}{}", front.origin(&label), page.path());
        if let Some(query) = page.query() {
            target.push('?');
            target.push_str(query);
        }
        if let Some(fragment) = page.fragment() {
            target.push('#');
            target.push_str(fragment);
        }
        Ok(target)
    }

    /// 第一次用到时才开监听；之后一直开着，浏览器那边的页面随时会回来要东西。
    fn front(&self) -> Result<Arc<BrowserFront>, CoreError> {
        let mut front = self.front.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(front) = front.as_ref() {
            return Ok(front.clone());
        }
        let runtime = self.runtime.clone();
        let routes: Arc<dyn BrowserRoutes> = self.routes.clone();
        let port = preferred_port(&self.routes.device_key);
        // UniFFI 和异步测试都可能调到这里，在帮手线程上 block_on，免得嵌套 tokio 的 block_on。
        let bound = std::thread::spawn(move || runtime.block_on(BrowserFront::bind(port, routes)))
            .join()
            .map_err(|_| front_failed("the bind thread panicked"))?
            .map_err(|e| front_failed(&e.to_string()))?;
        let bound = Arc::new(bound);
        *front = Some(bound.clone());
        Ok(bound)
    }
}

fn front_failed(error: &str) -> CoreError {
    CoreError::TransportUnavailable {
        reason: l10n!(CATALOG, language(), "browser-front-failed", error = error),
    }
}

/// 入口按标签现查：插件列表和连接都可能在浏览器开着时变化，不存一张会过期的表。
struct Routes {
    device_key: [u8; 32],
    home: watch::Receiver<HomeViewModel>,
    plugins: watch::Receiver<PluginsViewModel>,
    chat: watch::Receiver<ChatViewModel>,
}

impl Routes {
    fn server(&self) -> Option<String> {
        self.home.borrow().server_id.clone()
    }

    /// 声明了跨源隔离的页面：（标签用的 ID，网页的 origin）。插件用插件 ID，Bot 用 `bot:<插件>/<Bot>`。
    fn isolated_pages(&self) -> Vec<(String, String)> {
        let view = self.plugins.borrow();
        let plugins = view.installed.as_ref().map(|list| list.plugins.as_slice());
        let mut pages: Vec<(String, String)> = plugins
            .unwrap_or_default()
            .iter()
            .filter(|plugin| plugin.cross_origin_isolated)
            .filter_map(|plugin| {
                let url = Url::parse(plugin.web_url.as_deref()?).ok()?;
                Some((plugin.id.clone(), url.origin().ascii_serialization()))
            })
            .collect();
        drop(view);
        for bot in &self.chat.borrow().bots {
            let Some(BotWebView::Ready { page }) = bot.web.as_ref() else {
                continue;
            };
            if !page.cross_origin_isolated {
                continue;
            }
            let Ok(url) = Url::parse(&page.url) else {
                continue;
            };
            pages.push((
                format!("bot:{}/{}", page.bot.plugin_id, page.bot.bot),
                url.origin().ascii_serialization(),
            ));
        }
        pages
    }
}

impl BrowserRoutes for Routes {
    fn resolve(&self, wanted: &str) -> Result<BrowserRoute, BrowserRefusal> {
        let server = self.server().ok_or(BrowserRefusal::NotConnected)?;
        let (_, origin) = self
            .isolated_pages()
            .into_iter()
            .find(|(id, _)| same(&label(&self.device_key, &server, id), wanted))
            .ok_or(BrowserRefusal::UnknownAddress)?;
        let proxy = self
            .home
            .borrow()
            .proxy
            .clone()
            .ok_or(BrowserRefusal::NotConnected)?;
        let ip: IpAddr = proxy
            .host
            .parse()
            .map_err(|_| BrowserRefusal::NotConnected)?;
        Ok(BrowserRoute {
            origin,
            proxy: SocketAddr::new(ip, proxy.port),
            credential: ProxyCredential::from_parts(proxy.username, proxy.password),
        })
    }

    fn message(&self, refusal: BrowserRefusal) -> String {
        let id = match refusal {
            BrowserRefusal::UnknownAddress => "browser-unknown-address",
            BrowserRefusal::NotConnected => "browser-not-connected",
            BrowserRefusal::BadRequest => "browser-bad-request",
        };
        crate::l10n::text(id, &[])
    }
}

/// 浏览器地址里的标签：以设备密钥为键、对服务器和插件做 HMAC-SHA256，取前 128 位。标签是同机其他 App
/// 进隧道的唯一门槛，所以用带键的 MAC：知道一个插件的标签推不出别的插件的，也推不出设备密钥。
fn label(device_key: &[u8; 32], server: &str, plugin: &str) -> String {
    let digest = keyed(
        device_key,
        &[
            b"wonton browser origin\0",
            server.as_bytes(),
            &[0],
            plugin.as_bytes(),
        ],
    );
    digest[..LABEL_LEN / 2]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn preferred_port(device_key: &[u8; 32]) -> u16 {
    let digest = keyed(device_key, &[b"wonton browser port\0"]);
    PORT_BASE + u16::from_be_bytes([digest[0], digest[1]]) % PORT_SPAN
}

fn keyed(device_key: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(device_key).expect("HMAC accepts any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// 逐字节比较全部内容，不按前缀提前返回：从耗时上猜不出标签。
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

#[cfg(test)]
mod tests;
