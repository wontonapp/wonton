//! 核心和插件之间的入口：核心在插件网络里的别名（插件经它们访问核心转发的服务），以及核心访问插件的地址
//! （网页、模型网关、MCP 服务）。普通网络里连 [`names::service_host`] 的稳定域名（上游看到的 Host 就是它），
//! 宿主网络里的服务经 [`super::host_relay`] 的 unix 套接字访问。

mod address;
mod aliases;
mod endpoint;
mod probe;
mod resource;
mod routes;
mod webui;

pub(super) use aliases::{core_aliases, uses_sso};
pub use endpoint::{Endpoint, McpTarget};
pub(super) use probe::{PROBE_TIMEOUT, probe, status};
pub use resource::ResourceUpstreamError;

use super::health;
use super::health_gate::Wait;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::Manifest;
use super::models::ModelsRoute;
use super::names;
use super::secrets::Secrets;
use crate::l10n;

impl PluginManager {
    /// 安装、升级的容器起来之后：收存密钥，取网页、MCP 和模型网关的入口，声明了的入口取不到就是部署坏了，
    /// 最后等网页入口应一声。核对时容器本来就可能不在，所以入口函数给的是 `None`，这两条路要当错误。
    pub(in crate::plugin) async fn deploy_upstreams(
        &self,
        manifest: &Manifest,
        network: &str,
        secrets: &mut Secrets,
        wait: Wait,
        missing: super::secrets::CollectMissing,
        langs: &Languages,
    ) -> Result<(Option<Endpoint>, Option<ModelsRoute>, Option<McpTarget>), InstallError> {
        // 安装和升级到这里时一次性容器都已经退出（`start_containers` 等过）：它们该写下的密钥这会儿
        // 必须收到，不然 MCP 头填不出来。重启路径还在这一步收，没写下的下次再说。
        if super::secrets::collect(
            &self.engine,
            &self.cfg.instance,
            manifest,
            &self.bot_names(&manifest.id),
            secrets,
            missing,
            langs,
        )
        .await?
        {
            self.store.secrets().save(&manifest.id, secrets)?;
        }
        let upstream = self.webui_upstream(manifest, network).await?;
        let models = self.models_upstream(manifest, network).await?;
        let mcp = self.mcp_upstream(manifest, network, secrets).await?;
        check_present(
            upstream.as_ref(),
            models.as_ref(),
            mcp.as_ref(),
            manifest,
            &self.cfg.instance,
            network,
            langs,
        )?;
        if let (Some(upstream), Some(webui)) = (&upstream, &manifest.webui) {
            let container = self.webui_container(manifest, &webui.service);
            health::wait_webui(&self.http, upstream, &webui.path, &container, wait, langs).await?;
        }
        if super::bot_web_registry::has_bot_web(manifest) {
            for bot in self.store.bots().load(&manifest.id)?.bots.keys() {
                self.wait_bot_web(manifest, bot, wait).await?;
            }
        }
        Ok((upstream, models, mcp))
    }
}

/// 安装、升级刚确认过每个容器就绪，声明了的入口却取不到就是这次部署坏了。核对时容器本来就可能不在，
/// [`PluginManager::webui_upstream`] 这些给的是 `None`（原因在核对的 `problem` 里），这两条路要当错误。
pub(super) fn check_present(
    upstream: Option<&Endpoint>,
    models_route: Option<&ModelsRoute>,
    mcp_target: Option<&McpTarget>,
    manifest: &Manifest,
    instance: &names::Instance,
    network: &str,
    langs: &Languages,
) -> Result<(), InstallError> {
    let slug = manifest.id.slug();
    // 网页入口可以指向 Bot 服务，报的必须是默认 Bot 那份容器；其余入口校验保证不是 Bot 服务。
    let bots = super::bot_services::bot_services(manifest);
    let container = |service: &str| {
        if bots.contains(service) {
            instance.bot_container(&slug, service, super::bot_services::DEFAULT_BOT)
        } else {
            instance.container(&slug, service)
        }
    };
    let missing = |service: &str| {
        InstallError::Docker(l10n::text(
            langs,
            "plugin-install-upstream-container-missing",
            &[
                ("container", container(service).into()),
                ("network", network.into()),
            ],
        ))
    };
    if let Some(webui) = &manifest.webui
        && upstream.is_none()
    {
        return Err(missing(&webui.service));
    }
    if let Some(models) = manifest
        .provides
        .as_ref()
        .and_then(|provides| provides.models.as_ref())
        && models_route.is_none()
    {
        return Err(missing(&models.service));
    }
    if let Some(mcp) = &manifest.mcp
        && mcp_target.is_none()
    {
        return Err(missing(&mcp.service));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::package::PluginPackage;
    use super::*;
    use crate::plugin::languages::Languages;

    const BOT_WEB_MANIFEST: &str = "name: t
services:
  web:
    image: example/t
x-wonton:
  spec: '0.4'
  id: app.example.t
  version: 0.1.0
  kind: app
  title: {zh: 测试, en: Test}
  description: {zh: 测试, en: Test}
  webui:
    service: web
    port: 3000
  permissions: [matrix.bot]
  bots:
    services: [web]
";

    #[test]
    fn a_missing_bot_web_container_is_named_with_its_bot() {
        let langs = Languages::from_header(Some("zh-Hans"));
        let manifest = PluginPackage::parse(BOT_WEB_MANIFEST, &langs)
            .unwrap()
            .manifest;
        let error = check_present(
            None,
            None,
            None,
            &manifest,
            &names::Instance::default(),
            "wonton-app-example-t",
            &langs,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("容器 wonton-app-example-t--web--default 不存在"),
            "{error}"
        );
    }
}
