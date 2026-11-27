//! 生产里的曲库入口：插件列表给网页地址，家里的连接给本地代理。

use std::sync::{Arc, Mutex};

use url::Url;
use wonton_protocol::music::{MusicFailure, MusicProblem};
use wonton_protocol::{InstalledPlugin, PluginState, ProxySettings};

use super::{Endpoint, Libraries, lock};
use crate::home::HomeLink;
use crate::plugins::PluginsLink;

pub(crate) struct LiveLibraries {
    home: Arc<HomeLink>,
    plugins: Arc<PluginsLink>,
    client: Mutex<Option<(ProxySettings, reqwest::Client)>>,
}

impl LiveLibraries {
    pub(crate) fn new(home: Arc<HomeLink>, plugins: Arc<PluginsLink>) -> Arc<Self> {
        Arc::new(Self {
            home,
            plugins,
            client: Mutex::new(None),
        })
    }

    fn plugin(&self, plugin_id: &str) -> Option<InstalledPlugin> {
        self.plugins
            .view()
            .installed?
            .plugins
            .into_iter()
            .find(|plugin| plugin.id == plugin_id)
    }

    fn client(&self, proxy: &ProxySettings) -> Result<reqwest::Client, MusicProblem> {
        let mut cached = lock(&self.client);
        if let Some((settings, client)) = cached.as_ref()
            && settings == proxy
        {
            return Ok(client.clone());
        }
        let client = reqwest::Proxy::all(crate::chat::proxy_url(proxy))
            .and_then(|proxy| reqwest::Client::builder().no_proxy().proxy(proxy).build())
            .map_err(|error| problem(MusicFailure::Protocol, error.to_string()))?;
        *cached = Some((proxy.clone(), client.clone()));
        Ok(client)
    }
}

fn problem(failure: MusicFailure, details: String) -> MusicProblem {
    MusicProblem {
        failure,
        details: Some(details),
    }
}

impl Libraries for LiveLibraries {
    fn endpoint(&self, plugin_id: &str) -> Result<Endpoint, MusicProblem> {
        if let Some(reason) = self.home.tunnel_outage() {
            return Err(problem(MusicFailure::Transport, reason));
        }
        let proxy = self
            .home
            .view()
            .proxy
            .ok_or_else(|| problem(MusicFailure::Transport, "not connected".into()))?;
        let plugin = self.plugin(plugin_id).ok_or_else(|| {
            problem(
                MusicFailure::PluginNotReady,
                format!("{plugin_id} is not installed"),
            )
        })?;
        let web = plugin.web_url.ok_or_else(|| {
            problem(
                MusicFailure::PluginNotReady,
                format!("{plugin_id} has no web page"),
            )
        })?;
        // `web_url` 是点开整页的入口，可能带 `webui.path`；OpenSubsonic 的 `/rest` 和建号的 `/app/` 在源站根上。
        let mut base = Url::parse(&web)
            .map_err(|error| problem(MusicFailure::Protocol, format!("{web}: {error}")))?;
        base.set_path("/");
        base.set_query(None);
        base.set_fragment(None);
        Ok(Endpoint {
            client: self.client(&proxy)?,
            base,
        })
    }

    fn running(&self, plugin_id: &str) -> bool {
        self.plugin(plugin_id)
            .is_some_and(|plugin| matches!(plugin.status, PluginState::Running { .. }))
    }
}
