//! 插件管理当媒体源的提供方目录（[`crate::media::provider::ProviderDirectory`]）：核心自己是使用方，
//! 不走插件之间的连接记录，凭据由 [`crate::media::provider`] 按人签、存在核心那边。

use wonton_protocol::media::{MEDIA_CAPABILITY, MediaFailure, MediaFailureCode};

use super::{PluginId, PluginManager, PluginStatus, Provider, service};
use crate::media::provider::{Candidate, Pending, ProviderDirectory};
use crate::plugin::media_source::failure;
use crate::plugin::upstreams::Endpoint;

impl PluginManager {
    /// 在跑、而且声明的服务正在提供的 `media_source` 提供方。
    fn media_providers(&self) -> Vec<Provider> {
        self.capability_providers()
            .into_iter()
            .filter(|p| {
                matches!(
                    p.status,
                    PluginStatus::Running | PluginStatus::Degraded { .. }
                ) && service(&p.listed.manifest, MEDIA_CAPABILITY)
                    .is_some_and(|spec| p.serving.contains(&spec.service))
            })
            .collect()
    }

    fn media_provider(&self, id: &str) -> Result<Provider, MediaFailure> {
        self.media_providers()
            .into_iter()
            .find(|p| p.id.as_str() == id)
            .ok_or_else(|| {
                failure(
                    MediaFailureCode::PluginUnavailable,
                    format!("媒体源插件 {id} 没在跑"),
                )
            })
    }

    async fn media_endpoint(&self, provider: &str) -> Result<Endpoint, MediaFailure> {
        let found = self.media_provider(provider)?;
        #[cfg(test)]
        if let Some(endpoint) = self
            .dependencies
            .routes
            .read()
            .expect("测试能力路由锁中毒")
            .get(&(String::new(), provider.to_owned()))
            .cloned()
        {
            return Ok(endpoint);
        }
        let spec = service(&found.listed.manifest, MEDIA_CAPABILITY).ok_or_else(|| {
            failure(
                MediaFailureCode::PluginUnavailable,
                format!("媒体源插件 {provider} 没声明 {MEDIA_CAPABILITY}"),
            )
        })?;
        self.capability_endpoint(&found.listed.manifest, &spec.service, spec.port)
            .await
            .map_err(|e| failure(MediaFailureCode::PluginUnavailable, e.to_string()))?
            .ok_or_else(|| {
                failure(
                    MediaFailureCode::PluginUnavailable,
                    format!("媒体源插件 {provider} 的服务还没有地址"),
                )
            })
    }

    async fn media_command(
        &self,
        id: &str,
        issue: bool,
        name: &str,
        key: Option<&str>,
    ) -> Result<(), MediaFailure> {
        let provider = self.media_provider(id)?;
        let commands = service(&provider.listed.manifest, MEDIA_CAPABILITY)
            .and_then(|spec| spec.credentials.clone())
            .ok_or_else(|| {
                failure(
                    MediaFailureCode::PluginUnavailable,
                    format!("媒体源插件 {id} 没声明凭据命令"),
                )
            })?;
        let langs = self.owner_languages();
        self.capability_command(
            &provider,
            MEDIA_CAPABILITY,
            &commands,
            issue,
            name,
            key,
            &langs,
        )
        .await
        .map_err(|e| failure(MediaFailureCode::PluginUnavailable, e.to_string()))
    }
}

impl ProviderDirectory for PluginManager {
    fn candidates(&self) -> Vec<Candidate> {
        let default = self
            .dependencies
            .load()
            .ok()
            .and_then(|state| state.defaults.get(MEDIA_CAPABILITY).cloned());
        self.media_providers()
            .into_iter()
            .filter_map(|p| {
                let spec = service(&p.listed.manifest, MEDIA_CAPABILITY)?;
                Some(Candidate {
                    default: default.as_deref() == Some(p.id.as_str()),
                    id: p.id.to_string(),
                    operations: spec.operations.clone(),
                    kinds: spec.kinds.clone(),
                })
            })
            .collect()
    }

    fn endpoint<'a>(&'a self, provider: &'a str) -> Pending<'a, Endpoint> {
        Box::pin(self.media_endpoint(provider))
    }

    fn issue<'a>(&'a self, provider: &'a str, name: &'a str, key: &'a str) -> Pending<'a, ()> {
        Box::pin(self.media_command(provider, true, name, Some(key)))
    }

    fn revoke<'a>(&'a self, provider: &'a str, name: &'a str) -> Pending<'a, ()> {
        Box::pin(self.media_command(provider, false, name, None))
    }

    fn installed(&self, provider: &str) -> bool {
        PluginId::parse(provider).is_ok_and(|id| self.is_installed(&id))
    }
}
