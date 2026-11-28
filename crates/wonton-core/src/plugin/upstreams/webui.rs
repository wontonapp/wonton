//! 网页入口：核心访问的地址、容器名和探测状态码。

use super::super::PluginId;
use super::super::host_relay::Purpose;
use super::super::manager::{InstallError, PluginManager};
use super::super::manifest::Manifest;
use super::super::names;
use super::{Endpoint, PROBE_TIMEOUT, status};
use crate::http::HttpError;

impl PluginManager {
    /// 核心访问插件网页的地址。
    pub(in crate::plugin) async fn webui_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<Endpoint>, InstallError> {
        let Some(webui) = &manifest.webui else {
            return Ok(None);
        };
        let slug = manifest.id.slug();
        let per_bot = super::super::bot_web_registry::has_bot_web(manifest);
        let name = self.webui_container(manifest, &webui.service);
        if let Some(endpoint) = self
            .host_relay_endpoint(manifest, Purpose::Webui, &webui.service, webui.port)
            .await?
        {
            return Ok(Some(endpoint));
        }
        self.service_address(
            &name,
            &names::service_host(
                &slug,
                &if per_bot {
                    super::super::bot_services::bot_alias(&webui.service, "default")
                } else {
                    webui.service.clone()
                },
            ),
            webui.port,
            network,
        )
        .await
        .map(|address| address.map(Endpoint::Tcp))
    }

    /// 网页容器的名字：bot-web 的网页按 Bot 各一份，核心访问、诊断的是默认 Bot 那份。
    pub(in crate::plugin) fn webui_container(&self, manifest: &Manifest, service: &str) -> String {
        let slug = manifest.id.slug();
        if super::super::bot_web_registry::has_bot_web(manifest) {
            self.cfg
                .instance
                .bot_container(&slug, service, super::super::bot_services::DEFAULT_BOT)
        } else {
            self.cfg.instance.container(&slug, service)
        }
    }
    /// 网页探测的状态码（ADR-0033 的 `wonton__plugin_status`）：请求一次网页入口，和装完时的探测
    /// （[`super::super::health::wait_webui`]）用同一个地址（核心在插件网络里看到的 `主机:端口` 加网页路径）
    /// 和每请求 5 秒的时限；声明了 `webui.health` 时探健康路径。没有网页、还没部署到那一步的为空。
    pub async fn webui_status(&self, id: &PluginId) -> Option<Result<u16, HttpError>> {
        let (endpoint, path) = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            let entry = plugins.get(id)?;
            let webui = entry.listed.as_ref()?.manifest.webui.as_ref()?;
            (
                entry.upstream.clone()?,
                webui.health.clone().unwrap_or_else(|| webui.path.clone()),
            )
        };
        Some(status(&self.http, &endpoint, &path, PROBE_TIMEOUT).await)
    }
}
