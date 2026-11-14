//! 总线对 Agent 的 MCP 服务器：每个请求先认出调用方（网关放进请求扩展的 [`Caller`]），再交给 [`Bus`]。

use std::sync::Arc;

use hyper::http::request::Parts;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorData, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, SubscriptionFilter,
};
use rmcp::service::{NotificationContext, RequestContext, RoleServer, SubscriptionContext};

use super::Bus;
use crate::plugin::PluginId;
use crate::plugin::manifest::LocalizedText;
use wonton_protocol::{BotKeeper, ConfirmationOrigin, ORIGIN_META_KEY};

/// 调用总线的插件：网关按来源地址认出它（同模型网关），放进请求扩展。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub id: PluginId,
    /// 卡片上显示的名字，如 Hermes。
    pub name: String,
    /// 总线令牌认出的 Bot：私人 Bot 是归属人，共享 Bot 归全家（ADR-0045）。没有 Bot 记录的插件
    /// （不进聊天的工具插件）按来源地址认，是 `None`。
    pub bot: Option<BotIdentity>,
    /// 这次调用报的来源会话（`_meta` 的 [`ORIGIN_META_KEY`] 或请求头），没核对过；卡片上用的是核对后的，见
    /// [`super::audience`]。
    pub origin: Option<ConfirmationOrigin>,
    /// 这条调用属于哪条调用链（ADR-0061 第 10 节）：`wonton-session` 头里的会话在这个 Bot 的轮次里时是那条链。
    /// 会话不是这个 Bot 的就当没带，审计里记一行。
    pub chain: Option<String>,
    /// 核对过归属的 ACP 会话；交接去重和父会话路径用它，不以房间或链 ID 代替。
    pub session: Option<String>,
    pub request_id: Option<rmcp::model::RequestId>,
    /// 这次请求已按当前记录解析出的 AI 动手封顶（[`Bus::ai_cap`]）；内部入口直接构造的 `Caller` 留空，由总线再算。
    pub ai_level: Option<wonton_protocol::AiLevel>,
}

/// 令牌认出的 Bot。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotIdentity {
    /// 插件里的 Bot 名（插件规范的 `bots`）。
    pub bot: String,
    pub keeper: BotKeeper,
}

#[derive(Clone)]
pub struct Handler {
    bus: Arc<Bus>,
}

impl Caller {
    pub fn new(id: PluginId, title: Option<&LocalizedText>) -> Self {
        let name = title.map(super::cards::text).filter(|n| !n.is_empty());
        let name = name.unwrap_or_else(|| id.to_string());
        Self {
            id,
            name,
            bot: None,
            origin: None,
            chain: None,
            session: None,
            request_id: None,
            ai_level: None,
        }
    }

    /// 令牌认出的 Bot 的调用方。
    pub fn for_bot(
        id: PluginId,
        title: Option<&LocalizedText>,
        bot: String,
        keeper: BotKeeper,
    ) -> Self {
        let mut caller = Self::new(id, title);
        caller.bot = Some(BotIdentity { bot, keeper });
        caller
    }
}

/// 审计里的“会话认到了别人的轮次”：头里的会话不在这个 Bot 的链上时记一行。
#[derive(serde::Serialize)]
struct ForeignSession<'a> {
    caller: &'a str,
    event: &'static str,
    session: &'a str,
    /// 会话实际登记在哪个 Bot 的哪个会话上。
    bot: &'a str,
    chain: &'a str,
}

/// `wonton-session` 只在账本确认归属后采用；会话与链必须来自同一条登记。
fn chain_session(
    bus: &Bus,
    caller: &Caller,
    headers: Option<&hyper::HeaderMap>,
) -> Option<(String, String)> {
    let session = headers?
        .get(wonton_protocol::SESSION_HEADER)?
        .to_str()
        .ok()?;
    let chains = bus.agent_chains()?;
    let info = chains.ledger().session_chain(session)?;
    let own = caller.bot.as_ref().map(|bot| wonton_protocol::BotRef {
        plugin_id: caller.id.to_string(),
        bot: bot.bot.clone(),
    });
    if own.as_ref() == Some(&info.bot) {
        return Some((info.chain, session.to_owned()));
    }
    bus.audit.record(&ForeignSession {
        caller: caller.id.as_str(),
        event: "foreign_session",
        session,
        bot: &info.bot.bot,
        chain: &info.chain,
    });
    None
}

impl Handler {
    pub fn new(bus: Arc<Bus>) -> Self {
        Self { bus }
    }
}

fn caller(ctx: &RequestContext<RoleServer>) -> Result<Caller, ErrorData> {
    ctx.extensions
        .get::<Parts>()
        .and_then(|parts| parts.extensions.get::<Caller>())
        .cloned()
        .ok_or_else(|| ErrorData::invalid_request("请求没有经过核心网关", None))
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_tool_list_changed()
            .build();
        ServerConfig::new(capabilities)
            .with_server_info(Implementation::new("wonton-core", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Wonton 家里服务器的工具：核心自己的（wonton__ 开头，管理插件）和已装插件的（<插件>__<工具>）。\
                 安装、卸载等工具要主人在 Wonton App 里确认，调用会一直等到主人回答；wonton__set_profile 只改自己的资料，不用确认。\
                 工具结果里的文件写成编号：发给主人用 wonton__share_file，把返回的链接原样放进回复。",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut caller = caller(&ctx)?;
        caller.ai_level = Some(self.bus.ai_cap(&caller));
        Ok(ListToolsResult::with_all_items(
            self.bus.tools(&caller).await,
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let mut caller = caller(&ctx)?;
        let headers = ctx.extensions.get::<Parts>().map(|parts| &parts.headers);
        let meta = ctx.meta.0.0.get(ORIGIN_META_KEY);
        caller.origin = super::audience::reported_origin(meta, headers);
        if let Some((chain, session)) = chain_session(&self.bus, &caller, headers) {
            caller.chain = Some(chain);
            caller.session = Some(session);
        }
        caller.ai_level = Some(self.bus.ai_cap(&caller));
        caller.request_id = Some(ctx.id.clone());
        let cancelled = ctx.ct.clone().cancelled_owned();
        Ok(self.bus.call(&caller, request, cancelled).await.into())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        self.bus.track(context.peer);
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        let tools = requested.tools_list_changed == Some(true);
        Some(if tools {
            SubscriptionFilter::builder().tools_list_changed().build()
        } else {
            SubscriptionFilter::new()
        })
    }

    /// 2026-07-28 的订阅：插件表变了就通知一次，直到客户端取消订阅。
    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let mut changes = self.bus.plugins.changes();
        loop {
            tokio::select! {
                changed = changes.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                    if context.sink().notify_tool_list_changed().await.is_err() {
                        return Ok(());
                    }
                }
                () = context.cancelled() => return Ok(()),
            }
        }
    }
}
