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
use wonton_protocol::{AgentChainStatus, BotKeeper, ConfirmationOrigin, ORIGIN_META_KEY};

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
    /// 这轮消息的说话人（`X-Wonton-On-Behalf-Of` 或已报的轮次）。本机的人和 Bot 都给完整 MXID（联系人门要看服务器），
    /// 认不出的（含远端好友）是 [`TurnSender::Unknown`]；请求头和轮次都没有是 `None`，表示说不出替谁办事，
    /// 封顶按只读（[`Bus::ai_cap`]）。`None` 不能当成 Bot 归属人：缺头就拿到归属人的权限，等于身份升级。
    pub sender: Option<TurnSender>,
    /// 核对过归属的会话；权限卡片按它找状态，不以房间或链 ID 代替。
    pub session: Option<String>,
    pub request_id: Option<rmcp::model::RequestId>,
    /// 这次请求已按当前记录解析出的 AI 动手封顶（[`Bus::ai_cap`]）；内部入口直接构造的 `Caller` 留空，由总线再算。
    pub ai_level: Option<wonton_protocol::AiLevel>,
}

/// 这轮消息的说话人。认不出服务器或本机记录时用 `Unknown`，封顶按只读、联系人门按远端。
/// 本机 Bot 的账号也是 `Local`：封顶按它所在链的发起人算（ADR-0061 第 10 节）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnSender {
    Local(String),
    Unknown,
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
            sender: None,
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

/// 说话人来自请求头或已报轮次；报的人必须是这个 Bot 听的人。本服务器的人才给 [`TurnSender::Local`]，
/// 别家联系人按认不出。
pub(super) fn turn_sender(
    bus: &Bus,
    caller: &Caller,
    headers: Option<&hyper::HeaderMap>,
) -> Option<TurnSender> {
    let reported = headers.and_then(|headers| {
        headers
            .get(wonton_protocol::WONTON_ON_BEHALF_OF_HEADER)?
            .to_str()
            .ok()
    });
    if let Some(raw) = reported {
        return Some(if listens(bus, caller, raw) {
            local_sender(bus, raw)
        } else {
            TurnSender::Unknown
        });
    }
    // 没带请求头的 Bot 插件（如只报调用链的第三方 Bot）：先按报的会话找那一轮，找不到再看进行中的轮次。
    let bot = bot_ref(caller)?;
    let turns = bus.agent_chains()?.ledger().live_turns(&bot);
    if let (Some(session), Some(chain)) = (caller.session.as_deref(), caller.chain.as_deref())
        && let Some(turn) = turns
            .iter()
            .find(|turn| turn.chain == chain && turn.session == session)
    {
        return Some(local_sender(bus, &turn.sender));
    }
    // 并发的几轮分不清是哪一轮时取封顶最严的那轮，和没报说话人时的算法一致。一轮都没有时返回 `None`，
    // 由封顶按只读算，不回落到归属人。
    let mut lowest: Option<(wonton_protocol::AiLevel, TurnSender)> = None;
    for turn in turns {
        let sender = local_sender(bus, &turn.sender);
        let mut probe = caller.clone();
        probe.ai_level = None;
        probe.sender = Some(sender.clone());
        // 说话人本身可能是 Bot：封顶按这一轮的链算，而不是调用方碰巧带着的链。
        probe.chain = Some(turn.chain.clone());
        probe.session = Some(turn.session.clone());
        let level = bus.ai_cap(&probe);
        if lowest
            .as_ref()
            .is_none_or(|(current, _)| crate::ai_level::lower(level, *current) != *current)
        {
            lowest = Some((level, sender));
        }
    }
    lowest.map(|(_, sender)| sender)
}

fn bot_ref(caller: &Caller) -> Option<wonton_protocol::BotRef> {
    Some(wonton_protocol::BotRef {
        plugin_id: caller.id.to_string(),
        bot: caller.bot.as_ref()?.bot.clone(),
    })
}

fn listens(bus: &Bus, caller: &Caller, raw: &str) -> bool {
    let Some(bot) = bot_ref(caller) else {
        return false;
    };
    crate::gateway::agent_chains::bot_listens_to(
        &bus.plugins,
        bus.matrix(),
        bus.contacts(),
        &bot,
        raw,
    )
}

fn local_sender(bus: &Bus, raw: &str) -> TurnSender {
    let Some(rest) = raw.strip_prefix('@') else {
        return TurnSender::Unknown;
    };
    let Some((user, server)) = rest.split_once(':') else {
        return TurnSender::Unknown;
    };
    let Ok(people) = bus.people() else {
        return TurnSender::Unknown;
    };
    if user.is_empty() || !server.eq_ignore_ascii_case(&people.server_name) {
        return TurnSender::Unknown;
    }
    TurnSender::Local(raw.to_owned())
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
/// 链停了、会话也不在跑时照样把链交出去（记一条审计）：停链只撤得走当时挂着的卡片，
/// 之后到工具调用再开卡片时，靠这条链在绑卡那刻认出“已经停了”并当场撤回；从这里断开的话，
/// 卡片会一直等人答，主人批了还会真发出去。
pub(super) fn chain_session(
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
        let stopped = chains
            .ledger()
            .row(&info.chain)
            .is_some_and(|row| row.status == AgentChainStatus::Stopped && !info.live);
        if !stopped {
            return Some((info.chain, session.to_owned()));
        }
        bus.audit.record(&ForeignSession {
            caller: caller.id.as_str(),
            event: "stopped_session",
            session,
            bot: &info.bot.bot,
            chain: &info.chain,
        });
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
            .with_server_info(Implementation::new(
                "wonton-core",
                crate::build_info::version(),
            ))
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
        let headers = ctx.extensions.get::<Parts>().map(|parts| &parts.headers);
        if let Some((chain, session)) = chain_session(&self.bus, &caller, headers) {
            caller.chain = Some(chain);
            caller.session = Some(session);
        }
        caller.sender = turn_sender(&self.bus, &caller, headers);
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
        if caller.chain.is_none() && caller.bot.is_some() {
            tracing::debug!(
                tool = %request.name,
                header = headers
                    .and_then(|headers| headers.get(wonton_protocol::SESSION_HEADER))
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("-"),
                "会话没带链"
            );
        }
        caller.sender = turn_sender(&self.bus, &caller, headers);
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
