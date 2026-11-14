use tokio::sync::mpsc;
use wonton_protocol::BotRef;

use super::{Gateway, ProviderStream, Registration, dial};
use crate::mcp::ToolFailure;
use crate::plugin::{PluginId, dependencies::CapabilityRoute};

pub(crate) struct HandoffConnection {
    pub(crate) stream: ProviderStream,
    pub(crate) registration: Registration,
    pub(crate) cancel: mpsc::UnboundedReceiver<String>,
}

/// 总线已按真实调用方取得目标归属人的确认；不自报 HTTP 身份，也不开另一条链。
pub(crate) async fn connect_handoff(
    gateway: &Gateway,
    consumer: &PluginId,
    target: &BotRef,
    user: &str,
    chain: &str,
    route: CapabilityRoute,
) -> Result<HandoffConnection, ToolFailure> {
    if route.provider.as_str() != target.plugin_id
        || route.bot.as_ref().is_some_and(|bot| bot != target)
        || !gateway.bot_listens_to(target, user)
    {
        return Err(ToolFailure::new(
            "permission_denied",
            "目标 Bot 不听这条链代表的人",
        ));
    }
    let chains = gateway
        .agent_chains
        .as_ref()
        .ok_or_else(|| ToolFailure::new("service_unavailable", "调用链账本未启用"))?;
    let registry = gateway
        .agent_acp
        .as_ref()
        .ok_or_else(|| ToolFailure::new("service_unavailable", "ACP 连接注册表未启用"))?;
    let token = gateway
        .plugins
        .provider_token("agent_acp", &route.provider)
        .ok_or_else(|| ToolFailure::new("dependency_stopped", "目标提供方令牌未就绪"))?;
    let (cancel_tx, cancel) = mpsc::unbounded_channel();
    let registration = registry
        .register(
            consumer.as_str(),
            &crate::secret::random_token(),
            cancel_tx,
            chains.settings().concurrency(consumer.as_str()),
        )
        .ok_or_else(|| ToolFailure::new("busy", "调用方的 ACP 并发连接已满"))?;
    let langs = gateway.plugins.owner_languages();
    let stream = match dial(
        gateway,
        None,
        &route.endpoint,
        consumer,
        user,
        target,
        chain,
        chains.settings().limits(),
        &token,
        &langs,
    )
    .await
    {
        Ok(stream) => stream,
        Err(response) => return Err(dial_error(response).await),
    };
    Ok(HandoffConnection {
        stream,
        registration,
        cancel,
    })
}

async fn dial_error(response: axum::response::Response) -> ToolFailure {
    let status = response.status();
    let bytes = match axum::body::to_bytes(response.into_body(), 64 * 1024).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return ToolFailure::new(
                "dependency_stopped",
                format!("ACP 提供方握手失败（{status}），错误响应无法读取：{error}"),
            );
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return ToolFailure::new(
                "dependency_stopped",
                format!("ACP 提供方握手失败（{status}），错误响应无法解码：{error}"),
            );
        }
    };
    let kind = match value["error"]["kind"].as_str() {
        Some("permission_denied") => "permission_denied",
        Some("busy") => "busy",
        Some("stopped") => "stopped",
        Some("limit_reached") => "limit_reached",
        Some("invalid_input") => "invalid_input",
        _ => "dependency_stopped",
    };
    let detail = value["error"]["message"]
        .as_str()
        .unwrap_or("ACP 提供方拒绝了握手，但没有说明原因");
    ToolFailure::new(kind, detail)
}
