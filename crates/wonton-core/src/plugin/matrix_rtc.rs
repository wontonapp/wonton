//! `matrix_rtc` 能力的提供方（ADR-0096）：MSC4195 的 LiveKit 授权服务。使用方是 MatrixRTC 客户端，经网关
//! Matrix 对外地址下的 [`MATRIX_RTC_PREFIX`] 到达；同一时间只有一个提供方。
//!
//! 授权服务按标准的服务器发现去 `https://<服务器名>` 验 OpenID 令牌。插件网络里解析不了服务器名，所以核心在
//! 提供方的网络里给自己加上服务器名的别名，由 [`crate::matrix_rtc_listener`] 用 TLS 答其中两个路径。

use std::collections::BTreeMap;
use std::net::IpAddr;

use super::capabilities::{MATRIX_RTC, MATRIX_SERVER_NAME_VAR};
use super::entry::{Entry, PluginStatus};
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::{Manifest, PluginId};
use super::provides::ProviderService;
use super::upstreams::{Endpoint, core_aliases};
use crate::l10n;

pub use super::capabilities::MATRIX_RTC_PREFIX;

fn spec(manifest: &Manifest) -> Option<&ProviderService> {
    manifest
        .provides
        .as_ref()?
        .capabilities
        .get(MATRIX_RTC.name)
}

pub(in crate::plugin) fn provides_matrix_rtc(manifest: &Manifest) -> bool {
    spec(manifest).is_some()
}

/// 服务器名去掉端口，作为插件网络里的主机名（服务器名可以带端口，ADR-0009）。
pub fn server_host(server_name: &str) -> &str {
    if let Some(rest) = server_name.strip_prefix('[') {
        return rest.split_once(']').map_or(server_name, |(host, _)| host);
    }
    server_name
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(server_name, |(host, _)| host)
}

/// 网关转发 [`MATRIX_RTC_PREFIX`] 时找不到可用的提供方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatrixRtcUnavailable {
    /// 没装提供 `matrix_rtc` 的插件。
    NotInstalled,
    /// 装了，但没在运行，或者授权服务现在不可用；带上插件 ID 和原因。
    NotRunning { id: PluginId, detail: String },
}

impl PluginManager {
    /// 核心在这个插件网络里的全部别名：[`core_aliases`] 加上 `matrix_rtc` 提供方要的服务器名。
    pub(in crate::plugin) fn core_aliases_of(&self, manifest: &Manifest) -> Vec<String> {
        let mut aliases = core_aliases(&self.cfg.public, manifest);
        if provides_matrix_rtc(manifest)
            && let Some(server_name) = &self.matrix_server_name
        {
            let host = server_host(server_name).to_owned();
            if !aliases.contains(&host) {
                aliases.push(host);
            }
        }
        aliases
    }

    /// 注入给提供方的变量：Matrix 服务器名。
    pub(in crate::plugin) fn matrix_rtc_vars(
        &self,
        manifest: &Manifest,
    ) -> BTreeMap<String, String> {
        match (&self.matrix_server_name, provides_matrix_rtc(manifest)) {
            (Some(server_name), true) => {
                BTreeMap::from([(MATRIX_SERVER_NAME_VAR.to_owned(), server_name.clone())])
            }
            _ => BTreeMap::new(),
        }
    }

    /// 装或升级成 `matrix_rtc` 的提供方之前：核心要接着 Matrix（不然给不出服务器名），且别的插件没在提供它。
    pub(in crate::plugin) fn check_matrix_rtc_provider(
        &self,
        manifest: &Manifest,
        plugins: &BTreeMap<PluginId, Entry>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if !provides_matrix_rtc(manifest) {
            return Ok(());
        }
        if self.matrix_server_name.is_none() {
            return Err(InstallError::MatrixUnavailable(l10n::text(
                langs,
                "plugin-install-matrix-rtc-needs-matrix",
                &[],
            )));
        }
        let other = plugins.iter().find(|(id, entry)| {
            **id != manifest.id
                && entry
                    .listed
                    .as_ref()
                    .is_some_and(|listed| provides_matrix_rtc(&listed.manifest))
        });
        if let Some((id, _)) = other {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-matrix-rtc-conflict",
                &[("id", id.to_string().into())],
            )));
        }
        Ok(())
    }

    /// 来源地址是不是 `matrix_rtc` 提供方的插件网络（见 [`super::sources`]）。
    pub fn is_matrix_rtc_source(&self, ip: IpAddr) -> bool {
        let Some(source) = self.source(ip) else {
            return false;
        };
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(&source.id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| provides_matrix_rtc(&listed.manifest))
    }

    /// 网关转发 [`MATRIX_RTC_PREFIX`] 的目标。
    pub async fn matrix_rtc_route(&self) -> Result<(PluginId, Endpoint), MatrixRtcUnavailable> {
        let (id, manifest, service, port) = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            let Some((id, entry, spec)) = plugins.iter().find_map(|(id, entry)| {
                let spec = spec(&entry.listed.as_ref()?.manifest)?;
                Some((id, entry, spec))
            }) else {
                return Err(MatrixRtcUnavailable::NotInstalled);
            };
            let running = matches!(
                entry.status,
                PluginStatus::Running | PluginStatus::Degraded { .. }
            );
            if !running || !entry.serving.contains(&spec.service) {
                return Err(MatrixRtcUnavailable::NotRunning {
                    id: id.clone(),
                    detail: format!("{:?}，服务 {} 不可用", entry.status, spec.service),
                });
            }
            let listed = entry.listed.as_ref().expect("上面按清单找到的");
            (
                id.clone(),
                listed.manifest.clone(),
                spec.service.clone(),
                spec.port,
            )
        };
        #[cfg(test)]
        if let Some(endpoint) = self
            .dependencies
            .routes
            .read()
            .expect("测试能力路由锁中毒")
            .get(&(id.to_string(), MATRIX_RTC.name.to_owned()))
            .cloned()
        {
            return Ok((id, endpoint));
        }
        match self.capability_endpoint(&manifest, &service, port).await {
            Ok(Some(endpoint)) => Ok((id, endpoint)),
            Ok(None) => Err(MatrixRtcUnavailable::NotRunning {
                id,
                detail: format!("服务 {service} 没有地址"),
            }),
            Err(e) => Err(MatrixRtcUnavailable::NotRunning {
                id,
                detail: e.to_string(),
            }),
        }
    }

    /// 测试用：`matrix_rtc` 提供方（先 `register_for_test`）的授权服务在跑，直接指到这个地址。
    #[cfg(test)]
    pub(crate) fn matrix_rtc_route_for_test(&self, provider: &PluginId, endpoint: Endpoint) {
        {
            let mut plugins = self.plugins.write().expect("插件表锁中毒");
            let entry = plugins.get_mut(provider).expect("先 register_for_test");
            let service = entry
                .listed
                .as_ref()
                .and_then(|listed| spec(&listed.manifest))
                .map(|spec| spec.service.clone())
                .expect("清单里有 provides.matrix_rtc");
            entry.serving.insert(service);
        }
        self.dependencies
            .routes
            .write()
            .expect("测试能力路由锁中毒")
            .insert((provider.to_string(), MATRIX_RTC.name.to_owned()), endpoint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_names_lose_only_their_port() {
        assert_eq!(server_host("wonton.localhost"), "wonton.localhost");
        assert_eq!(server_host("rtc.test:18448"), "rtc.test");
        assert_eq!(server_host("[2001:db8::1]:8448"), "2001:db8::1");
        assert_eq!(server_host("[2001:db8::1]"), "2001:db8::1");
        assert_eq!(server_host("odd:"), "odd:");
    }
}
