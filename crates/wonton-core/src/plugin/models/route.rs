use super::{ModelsProvider, PLUGIN_MODELS_HOST, PluginId};

/// 核心转发模型请求的目标：当前的提供方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsRoute {
    pub id: PluginId,
    pub spec: ModelsProvider,
    /// 核心访问网关接口的地址 `host:port`。
    pub upstream: String,
    /// 执行凭据命令的容器。
    pub credentials_container: String,
}

/// 网关接口的基础地址：经核心在插件网络里的别名，不带末尾的 `/`。
pub(super) fn base_url(port: u16, path: &str) -> String {
    format!(
        "http://{PLUGIN_MODELS_HOST}:{port}{}",
        path.trim_end_matches('/')
    )
}

/// 模型网关现在的状态。
pub enum ProviderState {
    Absent,
    /// 装了，但没在运行（安装中、异常、卸载中）。
    NotReady(PluginId),
    Ready(Box<ModelsRoute>),
}
