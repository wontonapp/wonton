use wonton_protocol::ModelSettings;

use super::{
    InstallError, Languages, ModelsRoute, PluginId, PluginManager, ProviderState, gateway_absent,
    gateway_not_running, unavailable,
};

impl PluginManager {
    /// 当前可用的提供方：已安装、运行中的、声明了 `provides.models` 的插件。
    pub fn models_route(&self) -> Option<ModelsRoute> {
        match self.models_provider() {
            ProviderState::Ready(route) => Some(*route),
            _ => None,
        }
    }

    /// 模型设置页需要的网关网页和已知配置状态；配置状态来自核心曾成功接上的使用方。没有模型网关插件时如实报，
    /// 不退回 Agent 自己的网页：Agent 的模型在 App 里原生地接（ADR-0129）。
    pub(crate) async fn model_settings(
        &self,
        langs: &Languages,
    ) -> Result<ModelSettings, InstallError> {
        let has_configured_models = self.store.model_keys().has_connected_consumer()?;
        let route = self.provider(langs)?;
        Ok(ModelSettings {
            plugin_id: route.id.to_string(),
            web_url: self.model_web_url(&route.id),
            has_configured_models,
        })
    }

    /// 网关插件的网页地址，没有网页时为空。
    pub(crate) fn model_web_url(&self, id: &PluginId) -> Option<String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins.get(id).and_then(|entry| self.open_url(id, entry))
    }

    pub(super) fn provider(&self, langs: &Languages) -> Result<ModelsRoute, InstallError> {
        match self.models_provider() {
            ProviderState::Ready(route) => Ok(*route),
            ProviderState::NotReady(id) => {
                Err(unavailable(gateway_not_running(langs, id.as_str())))
            }
            ProviderState::Absent => Err(unavailable(gateway_absent(langs))),
        }
    }

    /// 等网关就绪：启动时核心并发装网关和默认 Bot，先装完的那个常常赶上网关还在起。到点还没起来就按实际
    /// 状态报错，调用方照旧安装、变量留空。没装（Absent）不用等。
    ///
    /// 调用方必须在拿 `dependencies.changing` **之前**等：起网关的 `start` 也要那把锁，锁里等只能等到超时。
    pub(in crate::plugin) async fn provider_when_ready(
        &self,
        langs: &Languages,
    ) -> Result<ModelsRoute, InstallError> {
        const INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
        for _ in 0..60 {
            match self.models_provider() {
                ProviderState::Ready(route) => return Ok(*route),
                ProviderState::NotReady(_) => tokio::time::sleep(INTERVAL).await,
                ProviderState::Absent => return Err(unavailable(gateway_absent(langs))),
            }
        }
        self.provider(langs)
    }
}
