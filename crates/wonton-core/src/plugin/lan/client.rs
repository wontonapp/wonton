//! 清单的 `lan.client`：第三方客户端（比如 Subsonic 播放器）在局域网上连这个插件用的地址和账号。写了它的局域网端口
//! 默认不开（见 [`super::webui`]），开关打开后详情页把地址和账号给主人。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use wonton_protocol::{LanClient, LanWebuiOffer};

use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::PluginManager;
use crate::plugin::manifest::{Manifest, ManifestError, PluginId};
use crate::plugin::proxy_auth;
use crate::plugin::secrets;

const HOST: &str = "{host}";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanClientSpec {
    /// 连接地址，`{host}` 换成端口开着的局域网地址，如 `http://{host}:4533`。
    pub address: String,
    /// 值模板，同 `webui.proxy`，但不能用 `{name}`。
    pub username: String,
    pub password: String,
}

pub(super) fn validate(
    manifest: &Manifest,
    client: &LanClientSpec,
    langs: &Languages,
) -> Result<(), ManifestError> {
    let invalid = |message: String| {
        ManifestError::Invalid(l10n::text(
            langs,
            "plugin-settings-lan-client-prefix",
            &[("message", message.into())],
        ))
    };
    if !client.address.contains(HOST) {
        return Err(invalid(l10n::text(
            langs,
            "plugin-settings-lan-client-address-host",
            &[("host", HOST.into())],
        )));
    }
    for template in [&client.username, &client.password] {
        // 账号不会是显示名，插件管理器也没有它。
        if template.contains("{name}") {
            return Err(invalid(l10n::text(
                langs,
                "plugin-settings-lan-client-template",
                &[("name", "{name}".into())],
            )));
        }
        proxy_auth::check_template(manifest, template, langs).map_err(invalid)?;
    }
    Ok(())
}

impl PluginManager {
    /// 主人看到的开关：开着时带上第三方客户端的连接信息。
    pub(in crate::plugin) async fn with_lan_client(
        &self,
        id: &PluginId,
        offer: Option<LanWebuiOffer>,
    ) -> Option<LanWebuiOffer> {
        let mut offer = offer?;
        if offer.enabled {
            match self.lan_client(id).await {
                Ok(client) => offer.client = client,
                Err(problem) => {
                    tracing::error!(plugin = %id, %problem, "取不到第三方客户端的连接信息");
                    offer.client_problem = Some(problem);
                }
            }
        }
        Some(offer)
    }

    async fn lan_client(&self, id: &PluginId) -> Result<Option<LanClient>, String> {
        let langs = self.owner_languages();
        let listed = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(id).and_then(|entry| entry.listed.clone())
        };
        let Some(listed) = listed else {
            return Ok(None);
        };
        let manifest = &listed.manifest;
        let Some(lan) = &manifest.lan else {
            return Ok(None);
        };
        let Some(client) = &lan.client else {
            return Ok(None);
        };
        let stored = self.store.secrets().load(id).map_err(|e| e.to_string())?;
        let stored = stored
            .ok_or_else(|| l10n::text(&langs, "plugin-settings-lan-client-secret-missing", &[]))?;
        let generated = secrets::generated_values(manifest, &stored, &langs)?;
        let owner = self.cfg.profile.owner();
        let fill = |template| proxy_auth::fill_template(template, &owner, &generated, &langs);
        let mut hosts = BTreeSet::new();
        let slug = id.slug();
        for port in &lan.ports {
            let name = self.cfg.instance.container(&slug, &port.service);
            let info = self.engine.inspect_container(&name).await;
            let info = info.map_err(|e| {
                l10n::text(
                    &langs,
                    "plugin-settings-lan-client-container",
                    &[("container", name.into()), ("error", e.to_string().into())],
                )
            })?;
            let bindings = info
                .and_then(|info| info.host_config)
                .and_then(|host| host.port_bindings)
                .and_then(|mut all| all.remove(&port.key()).flatten())
                .unwrap_or_default();
            hosts.extend(bindings.into_iter().filter_map(|b| b.host_ip));
        }
        if hosts.is_empty() {
            return Err(l10n::text(
                &langs,
                "plugin-settings-lan-client-not-published",
                &[],
            ));
        }
        // 启动器声明了宣告地址时，端口绑在通配地址上，容器记录里的地址不能给用户；连的是宣告地址。
        if let Some(announce) = &self.lan.announce {
            hosts = announce.iter().map(|a| a.ip.to_string()).collect();
        }
        Ok(Some(LanClient {
            addresses: hosts
                .iter()
                .map(|host| client.address.replace(HOST, &bracketed(host)))
                .collect(),
            username: fill(&client.username)?,
            password: fill(&client.password)?,
        }))
    }
}

fn bracketed(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}
