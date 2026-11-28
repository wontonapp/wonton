//! 假数据入口里的家庭音乐库：一台写死曲目的 OpenSubsonic 假曲库跑在本机回环地址上，真的浏览层
//! （`music_library`）照常去讲，原生音乐页不连家里也能走屏。插件声明了 `browse: opensubsonic`，点开是原生界面，
//! 胶囊菜单里的“网页版”和别的假应用一样停在骨架（没有隧道）。

mod catalog;
mod cover;
mod data;
mod server;

use std::path::Path;
use std::sync::{Arc, OnceLock};

use url::Url;
use wonton_protocol::media::MediaBrowse;
use wonton_protocol::music::MusicProblem;
use wonton_protocol::{
    InstalledPlugin, NativePage, PluginHealth, PluginState, TabIcon, TabLayout, TabPage, TabRef,
    TabState,
};

use super::plugins::fake_plugin;
use super::tabs::web_url;
use crate::music_library::{Endpoint, Libraries, MusicLink};

pub(super) const MUSIC_PLUGIN: &str = "org.navidrome.navidrome";
const MUSIC_TITLE: &str = "Navidrome";

/// 假曲库的运行时和浏览层：整个进程一份，第一次打开音乐页时起。
static LIBRARY: OnceLock<(tokio::runtime::Runtime, Arc<MusicLink>)> = OnceLock::new();

/// 原生音乐页在假数据入口下用的浏览层。
pub(in crate::client) fn music_link(root: &Path) -> Arc<MusicLink> {
    LIBRARY
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("ui-fake-music")
                .enable_all()
                .build()
                .expect("the fake music library needs a runtime");
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
                .expect("the fake music library needs a loopback port");
            let port = listener.local_addr().expect("a bound port").port();
            listener
                .set_nonblocking(true)
                .expect("a non-blocking listener");
            runtime.spawn(server::serve(listener));
            let base = Url::parse(&format!("http://127.0.0.1:{port}/")).expect("a loopback URL");
            let link = MusicLink::new(
                runtime.handle().clone(),
                Arc::new(FakeLibrary { base }),
                root.join("ui-fake-music-covers"),
            );
            (runtime, link)
        })
        .1
        .clone()
}

struct FakeLibrary {
    base: Url,
}

impl Libraries for FakeLibrary {
    fn endpoint(&self, _plugin_id: &str) -> Result<Endpoint, MusicProblem> {
        Ok(Endpoint {
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("a plain HTTP client"),
            base: self.base.clone(),
        })
    }

    fn running(&self, _plugin_id: &str) -> bool {
        true
    }
}

/// 应用网格里的音乐库。
pub(super) fn plugin() -> InstalledPlugin {
    let mut plugin = fake_plugin(
        MUSIC_TITLE,
        MUSIC_PLUGIN,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Some(web_url(MUSIC_PLUGIN)),
    );
    plugin.media_browse = Some(MediaBrowse::OpenSubsonic);
    plugin
}

/// 打开音乐库的入口是原生界面，“网页版”是网页。
pub(super) fn page(tab: &TabRef) -> Option<TabPage> {
    let native = match tab {
        TabRef::Plugin { plugin_id, page } if plugin_id == MUSIC_PLUGIN => {
            page.is_none().then_some(NativePage::Music)
        }
        TabRef::PluginPath { plugin_id, .. } if plugin_id == MUSIC_PLUGIN => None,
        _ => return None,
    };
    Some(TabPage {
        title: MUSIC_TITLE.into(),
        icon: TabIcon::Plugin { icon: None },
        url: Some(web_url(MUSIC_PLUGIN)),
        layout: TabLayout::default(),
        state: TabState::Ready,
        reload: 0,
        opens_in_browser: false,
        native,
    })
}
