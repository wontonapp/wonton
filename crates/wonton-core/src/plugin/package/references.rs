//! 清单里点名的服务（网页、MCP、模型网关、密钥收集、Bot、GPU、局域网和隧道端口）都要在 compose 里，
//! 而且不能把全插件一份的入口指到 Bot 服务上。

use super::errors::{service_error, unknown_service_error};
use super::{PackageError, PluginPackage};
use crate::l10n::text;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::{Manifest, SecretSpec};

impl PluginPackage {
    /// `devices` 里的服务引用，按类给字段名。
    fn device_service_refs(
        manifest: &Manifest,
    ) -> impl Iterator<Item = (&'static str, Option<&String>)> {
        manifest
            .devices
            .iter()
            .flat_map(|devices| devices.classes())
            .flat_map(move |(class, slots)| {
                slots
                    .iter()
                    .map(move |slot| (class.field(), Some(&slot.service)))
            })
    }

    pub(super) fn check_manifest_references(&self, langs: &Languages) -> Result<(), PackageError> {
        let models = self
            .manifest
            .provides
            .as_ref()
            .and_then(|p| p.models.as_ref());
        let credentials = models.and_then(|m| m.credentials.service.as_ref());
        let egress = self
            .manifest
            .provides
            .as_ref()
            .and_then(|p| p.egress_proxy.as_ref());
        let refs = [
            (
                "webui.service",
                // 静态网页的入口没有服务（清单校验已保证 compose 写了）。
                self.manifest
                    .webui
                    .as_ref()
                    .map(|w| &w.service)
                    .filter(|s| !s.is_empty()),
            ),
            (
                "mcp.service",
                self.manifest.mcp.as_ref().map(|m| &m.service),
            ),
            ("provides.models.service", models.map(|m| &m.service)),
            ("provides.models.credentials.service", credentials),
            ("provides.egress_proxy.service", egress.map(|p| &p.service)),
        ];
        let collects = self
            .manifest
            .secrets
            .values()
            .filter_map(|spec| match spec {
                SecretSpec::Collect(collect) => {
                    Some(("secrets.collect.service", Some(&collect.service)))
                }
                SecretSpec::Generate(_) | SecretSpec::Input(_) => None,
            });
        let bot_services = self.manifest.bots.iter().flat_map(|b| &b.services);
        let bots = bot_services
            .clone()
            .map(|service| ("bots.services", Some(service)));
        let gpu = self
            .manifest
            .gpu
            .iter()
            .flat_map(|g| &g.services)
            .map(|service| ("gpu.services", Some(service)));
        let lan = self
            .manifest
            .lan
            .iter()
            .flat_map(|l| &l.ports)
            .map(|port| ("lan.ports", Some(&port.service)));
        let tunnel = self
            .manifest
            .tunnel
            .iter()
            .flat_map(|t| &t.ports)
            .map(|port| ("tunnel.ports", Some(&port.service)));
        for (field, service) in refs
            .iter()
            .cloned()
            .chain(collects)
            .chain(bots)
            .chain(gpu)
            .chain(Self::device_service_refs(&self.manifest))
            .chain(lan)
            .chain(tunnel)
        {
            if let Some(service) = service
                && !self.services.contains_key(service)
            {
                return Err(unknown_service_error(langs, field, service.clone()));
            }
        }
        // 网页入口连默认 Bot；MCP 和 provides 仍是全插件一份的入口。
        let bot_entry = refs
            .iter()
            .filter(|(field, _)| *field != "webui.service")
            .find_map(|(field, service)| {
                service
                    .filter(|s| bot_services.clone().any(|b| b == *s))
                    .map(|s| (*field, s))
            });
        if let Some((field, service)) = bot_entry {
            return Err(service_error(
                langs,
                service,
                text(
                    langs,
                    "plugin-install-pkg-ref-bot-service",
                    &[("field", (*field).into())],
                ),
            ));
        }
        self.check_egress_service(egress, langs)?;
        self.check_capability_services(langs)?;
        super::gpu_images::check(self, langs)?;
        self.check_clients_references(langs)
    }

    /// 出网插件的服务不能设 `network_mode`：出口容器没法和它共享网络。
    fn check_egress_service(
        &self,
        provider: Option<&crate::plugin::provides::EgressProxyProvider>,
        langs: &Languages,
    ) -> Result<(), PackageError> {
        if let Some(provider) = provider
            && self.services[&provider.service]
                .as_object()
                .is_some_and(|service| service.contains_key("network_mode"))
        {
            return Err(service_error(
                langs,
                &provider.service,
                text(langs, "plugin-install-pkg-egress-network-mode", &[]),
            ));
        }
        Ok(())
    }

    /// `clients` 里的服务要在 compose 里；引用了 `WONTON_EXTERNAL_URL` 时要开得出入口。
    fn check_clients_references(&self, langs: &Languages) -> Result<(), PackageError> {
        let clients = self.manifest.clients.as_ref();
        let http = clients
            .and_then(|c| c.http.as_ref())
            .map(|http| ("clients.http.service", &http.service));
        let tcp = clients
            .and_then(|c| c.tcp.as_ref())
            .into_iter()
            .flatten()
            .map(|entry| ("clients.tcp", &entry.service));
        for (field, service) in http.into_iter().chain(tcp) {
            if !self.services.contains_key(service) {
                return Err(unknown_service_error(langs, field, service.clone()));
            }
        }
        if let Some((field, service)) = http
            && self
                .manifest
                .bots
                .iter()
                .flat_map(|bots| &bots.services)
                .any(|bot_service| bot_service == service)
        {
            return Err(service_error(
                langs,
                service,
                text(
                    langs,
                    "plugin-install-pkg-ref-bot-service",
                    &[("field", field.into())],
                ),
            ));
        }
        crate::plugin::manifest_clients::validate_external_url(
            &self.manifest,
            crate::plugin::manifest_clients::references_external_url(&self.services),
            langs,
        )
        .map_err(PackageError::Manifest)
    }
}
