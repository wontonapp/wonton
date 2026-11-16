//! `agent_acp` 的转发（ADR-0061 第 5 块）：核心终结使用方经 `agent_acp.wonton.internal/acp` 的
//! WebSocket，逐帧转发给提供方。不走 [`super::proxy`] 的字节管道：核心要数链上的 `session/prompt`
//! （计轮）、停链时向提供方注入 `session/cancel`（能力表的转发约束见 plugin/capabilities.rs 开头）。
//!
//! 建连顺序：认使用方（插件网络来源）→ 读使用方自报的发起 Bot（ADR-0053）→ 找提供方（Bot 作路由键）
//! → 定目标 Bot → 采纳说话人 → 核对目标 Bot 听人 → 提供方令牌 → 并发名额 → 预发链 → 连提供方
//! → 回 101 开转发。每个 session/new 再按载体核对归链；使用方的 depth 不直接透传。

mod concurrency;
mod frames;
mod handoff;
mod headers;
mod identity;
mod sessions;

use std::sync::Arc;
use std::time::Duration;

use aws_lc_rs::digest;
use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CONNECTION, HOST, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use tokio::sync::mpsc;
use tokio_websockets::{ClientBuilder, Limits, Message, ServerBuilder, WebSocketStream};
use wonton_l10n::Arg;
use wonton_protocol::{ChainLimits, ConfirmationOrigin};

use super::agent_chains::AgentChains;
use super::{Gateway, api, client_addr, proxy};
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::capabilities::AGENT_ACP;
use crate::plugin::languages::Languages;
use crate::plugin::upstreams::Endpoint;

/// 提供方握手的期限：升级该秒级完成，挂起按不可用处理。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// 提供方一侧的 WebSocket：升级握手已完成，直接收发帧。
type ProviderStream = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;

pub(crate) use concurrency::Registration;
pub use concurrency::Registry;
pub(crate) use handoff::connect_handoff;

/// 停链的取消方：注册表按账本会话键找到活连接，发 `session/cancel`。
impl super::agent_chains::AgentAcpCancel for Registry {
    fn cancel(&self, session: &str) {
        self.cancel(session);
    }
}

/// 建连检查都过后的上下文：转发要用的全部事实。
struct Prepared {
    sessions: sessions::Sessions,
    endpoint: Endpoint,
    provider: ProviderStream,
    registration: concurrency::Registration,
    cancel_rx: mpsc::UnboundedReceiver<String>,
}

pub(super) async fn request(gateway: &Gateway, mut req: Request) -> Response {
    let langs = api::languages(req.headers());
    let Some(source) = client_addr(&req).and_then(|addr| gateway.plugins.source(addr.ip())) else {
        return api::error_response(
            StatusCode::FORBIDDEN,
            "unauthenticated",
            &l10n::text(&langs, "gateway-plugin-network-only", &[]),
        );
    };
    if !is_websocket_upgrade(&req) {
        return api::error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &l10n::text(&langs, "gateway-agent-acp-websocket-only", &[]),
        );
    }
    let prepared = match prepare(gateway, &mut req, &source.id, &langs).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let reply = match upgrade_reply(&req) {
        Ok(reply) => reply,
        Err(response) => return response,
    };
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(pump(prepared, on_upgrade));
    reply
}

/// 建连的全部检查和资源占用。Err 的响应直接回给使用方。
#[expect(clippy::result_large_err)]
async fn prepare(
    gateway: &Gateway,
    req: &mut Request,
    consumer: &PluginId,
    langs: &Languages,
) -> Result<Prepared, Response> {
    let initiator = identity::initiator_bot(gateway, consumer, req.headers(), langs)?;
    let route = gateway
        .plugins
        .capability_route(consumer, AGENT_ACP.name, langs, Some(&initiator))
        .await
        .map_err(|error| route_error(consumer, error, langs))?;
    let target = identity::target_bot(gateway, &route.provider, route.bot.as_ref(), langs)?;
    let user = identity::chain_user(gateway, &initiator, req.headers(), langs)?;
    if !gateway.bot_listens_to(&target, &user) {
        return Err(refused(
            langs,
            "gateway-agent-acp-target-not-listened",
            &[("bot", target.bot.as_str().into())],
        ));
    }
    let token = gateway
        .plugins
        .provider_token(AGENT_ACP.name, &route.provider)
        .ok_or_else(|| {
            unavailable(
                langs,
                "gateway-agent-acp-token-missing",
                &[("provider", route.provider.as_str().into())],
            )
        })?;
    let chains = chains_of(gateway, langs)?;
    let limits = chains.settings().limits();
    let concurrency = chains.settings().concurrency(consumer.as_str());
    let (cancel_tx, cancel_rx) = mpsc::unbounded_channel();
    let connection = connection_key();
    let registration = gateway
        .agent_acp
        .as_ref()
        .and_then(|registry| {
            registry.register(consumer.as_str(), &connection, cancel_tx, concurrency)
        })
        .ok_or_else(|| busy(langs))?;
    let grant = chains
        .ledger()
        .open_acp_connection(
            &user,
            ConfirmationOrigin {
                room_id: String::new(),
                thread_id: None,
            },
            target.clone(),
            &connection,
        )
        .map_err(super::agent_chains::ledger_error)?;
    let sessions = sessions::Sessions::new(
        chains.ledger().clone(),
        connection.clone(),
        consumer.to_string(),
        initiator,
    );
    let host = req.headers().get(HOST).cloned();
    let provider = match dial(
        gateway,
        host,
        &route.endpoint,
        consumer,
        &user,
        &target,
        &grant.chain,
        limits,
        &token,
        langs,
    )
    .await
    {
        Ok(provider) => provider,
        Err(error) => {
            drop(sessions);
            let _ = chains.ledger().end_chain(&grant.chain);
            return Err(error);
        }
    };
    tracing::info!(chain = %grant.chain, "agent_acp 连接开了一条新链");
    Ok(Prepared {
        sessions,
        endpoint: route.endpoint,
        provider,
        registration,
        cancel_rx,
    })
}

/// 转发泵：使用方的升级完成后，两条流逐帧对接直到任一侧结束。
async fn pump(
    Prepared {
        mut sessions,
        endpoint,
        provider,
        mut registration,
        mut cancel_rx,
    }: Prepared,
    on_upgrade: hyper::upgrade::OnUpgrade,
) {
    let limits = ws_limits();
    let consumer_ws = match on_upgrade.await {
        Ok(upgraded) => ServerBuilder::new()
            .limits(limits)
            .serve(TokioIo::new(upgraded)),
        Err(error) => {
            tracing::warn!(%error, "agent_acp 使用方升级没完成");
            return;
        }
    };
    let (mut consumer_tx, mut consumer_rx) = consumer_ws.split();
    let (mut provider_tx, mut provider_rx) = provider.split();
    let mut cancel_open = true;
    loop {
        tokio::select! {
            signal = cancel_rx.recv(), if cancel_open => match signal {
                // 停链（ADR-0061 第 10 节）：对每个活着的会话发取消；未决请求由双方按 ACP 自己回 cancelled。
                Some(key) => {
                    for session in sessions.cancelling(&key) {
                        if provider_tx
                            .send(Message::text(frames::cancel_notification(session)))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                None => cancel_open = false,
            },
            frame = consumer_rx.next() => match frame {
                Some(Ok(message)) => {
                    if !forward_consumer(
                        &mut consumer_tx,
                        &mut provider_tx,
                        &mut sessions,
                        &mut registration,
                        message,
                    )
                    .await
                    {
                        break;
                    }
                }
                Some(Err(error)) => {
                    tracing::warn!(%error, "agent_acp 使用方一侧的流出错");
                    break;
                }
                None => break,
            },
            frame = provider_rx.next() => match frame {
                Some(Ok(message)) => {
                    if !forward_provider(&mut consumer_tx, &mut provider_tx, &mut sessions, message).await {
                        break;
                    }
                }
                Some(Err(error)) => {
                    tracing::warn!(provider = ?endpoint, %error, "agent_acp 提供方一侧的流出错");
                    break;
                }
                None => break,
            },
        }
    }
    drop(registration);
}

/// 使用方 → 提供方：建会话先核对并注入链载体，prompt 按会话记轮；拒绝不转给提供方。
async fn forward_consumer(
    consumer_tx: &mut (impl futures_util::Sink<Message, Error = tokio_websockets::Error> + Unpin),
    provider_tx: &mut (impl futures_util::Sink<Message, Error = tokio_websockets::Error> + Unpin),
    sessions: &mut sessions::Sessions,
    registration: &mut concurrency::Registration,
    message: Message,
) -> bool {
    if message.is_text() {
        let Some(mut frame) = message.as_text().and_then(frames::parse) else {
            tracing::warn!("agent_acp 收到不是 JSON-RPC 的文本帧，断开");
            return false;
        };
        let is_new = frame["method"] == "session/new";
        let checked = if is_new {
            sessions
                .prepare_new(&mut frame, registration)
                .map(|()| true)
        } else if frame["method"] == "session/load" {
            sessions.prepare_load(&frame, registration).map(|()| false)
        } else if frame["method"] == "session/prompt" {
            sessions.prepare_prompt(&mut frame)
        } else {
            Ok(false)
        };
        let rewritten = match checked {
            Ok(rewritten) => rewritten,
            Err(error) => {
                let id = frame.get("id").unwrap_or(&serde_json::Value::Null);
                return consumer_tx
                    .send(Message::text(frames::error_reply(id, &error)))
                    .await
                    .is_ok();
            }
        };
        let message = if rewritten {
            Message::text(frame.to_string())
        } else {
            message
        };
        return provider_tx.send(message).await.is_ok();
    }
    if message.is_close() {
        let _ = provider_tx.send(Message::close(None, "")).await;
        return false;
    }
    // ping/pong 由 tokio-websockets 自动应答，帧不再转发；二进制帧不在契约里，按协议错误断开。
    message.is_ping() || message.is_pong()
}

/// 提供方 → 使用方：原样转发，顺手记下会话 ID（停链时逐个取消）。
async fn forward_provider(
    consumer: &mut (impl futures_util::Sink<Message, Error = tokio_websockets::Error> + Unpin),
    provider: &mut (impl futures_util::Sink<Message, Error = tokio_websockets::Error> + Unpin),
    sessions: &mut sessions::Sessions,
    message: Message,
) -> bool {
    if message.is_text() {
        if let Some(frame) = message.as_text().and_then(frames::parse)
            && let Some(session) = sessions.observe_response(&frame)
            && provider
                .send(Message::text(frames::cancel_notification(&session)))
                .await
                .is_err()
        {
            return false;
        }
        return consumer.send(message).await.is_ok();
    }
    if message.is_close() {
        let _ = consumer.send(Message::close(None, "")).await;
        return false;
    }
    message.is_ping() || message.is_pong()
}

/// 连提供方：核心发的升级请求只带核心注入的头（headers::provider_handshake），
/// 使用方的握手头一概不带（Host 除外）。每条连接一次拨号，转发期间不复用。
#[expect(clippy::too_many_arguments, clippy::result_large_err)]
async fn dial(
    gateway: &Gateway,
    host: Option<HeaderValue>,
    endpoint: &Endpoint,
    consumer: &PluginId,
    user: &str,
    target: &wonton_protocol::BotRef,
    chain_id: &str,
    limits: ChainLimits,
    token: &str,
    langs: &Languages,
) -> Result<ProviderStream, Response> {
    let path = "/acp";
    let authority = match endpoint {
        Endpoint::Tcp(authority) => format!("http://{authority}{path}"),
        Endpoint::Unix(_) => format!("http://localhost{path}"),
    };
    let mut key = [0u8; 16];
    getrandom::fill(&mut key).expect("操作系统随机数源不可用");
    let handshake =
        headers::provider_handshake(consumer.as_str(), user, chain_id, target, limits, token);
    let mut builder = axum::http::Request::builder()
        .method(Method::GET)
        .uri(authority)
        .header(CONNECTION, "Upgrade")
        .header(UPGRADE, "websocket")
        .header("sec-websocket-version", "13")
        .header(SEC_WEBSOCKET_KEY, STANDARD.encode(key));
    if let Some(host) = &host {
        builder = builder.header(HOST, host.clone());
    }
    for (name, value) in handshake.iter() {
        builder = builder.header(name, value);
    }
    let request = builder.body(Body::empty()).expect("升级请求的构造不会失败");
    let response = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        match endpoint {
            Endpoint::Tcp(_) => gateway.upstream_http.send(request).await,
            Endpoint::Unix(socket) => proxy::unix_send(socket, request, None).await,
        }
    })
    .await;
    let mut response = match response {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            return Err(unreachable_provider(langs, &error.to_string()));
        }
        Err(_) => {
            return Err(unreachable_provider(langs, "握手超时"));
        }
    };
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        return Err(unreachable_provider(
            langs,
            &format!("提供方回了 {}", response.status()),
        ));
    }
    let upgraded = match hyper::upgrade::on(&mut response).await {
        Ok(upgraded) => upgraded,
        Err(error) => {
            return Err(unreachable_provider(langs, &error.to_string()));
        }
    };
    Ok(ClientBuilder::new()
        .limits(ws_limits())
        .take_over(TokioIo::new(upgraded)))
}

/// 回给使用方的 101：按它带来的 Sec-WebSocket-Key 算 accept。
#[expect(clippy::result_large_err)]
fn upgrade_reply(req: &Request) -> Result<Response, Response> {
    let key = req
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            api::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                "缺少 Sec-WebSocket-Key",
            )
        })?;
    let mut hash = digest::Context::new(&digest::SHA1_FOR_LEGACY_USE_ONLY);
    hash.update(key.as_bytes());
    hash.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept = STANDARD.encode(hash.finish().as_ref());
    Ok(Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(CONNECTION, "Upgrade")
        .header(UPGRADE, "websocket")
        .header(SEC_WEBSOCKET_ACCEPT, accept)
        .body(Body::empty())
        .expect("101 响应的构造不会失败"))
}

fn ws_limits() -> Limits {
    Limits::default().max_payload_len(Some(frames::MAX_FRAME_BYTES))
}

fn route_error(
    consumer: &PluginId,
    error: crate::plugin::dependencies::CapabilityAccessError,
    langs: &Languages,
) -> Response {
    match error {
        crate::plugin::dependencies::CapabilityAccessError::Denied => refused(
            langs,
            "gateway-capability-permission",
            &[
                ("id", consumer.as_str().into()),
                ("capability", AGENT_ACP.name.into()),
            ],
        ),
        crate::plugin::dependencies::CapabilityAccessError::Unavailable(detail) => {
            api::error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "dependency_stopped",
                &detail,
            )
        }
    }
}

fn unreachable_provider(langs: &Languages, detail: &str) -> Response {
    api::error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_stopped",
        &l10n::text(
            langs,
            "gateway-agent-acp-provider-unreachable",
            &[("detail", detail.into())],
        ),
    )
}

/// 这台核心没接调用链：如实报不可用。
#[expect(clippy::result_large_err)]
fn chains_of(gateway: &Gateway, langs: &Languages) -> Result<Arc<AgentChains>, Response> {
    gateway.agent_chains.clone().ok_or_else(|| {
        api::error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "chains_unavailable",
            &l10n::text(langs, "gateway-chains-disabled", &[]),
        )
    })
}

fn connection_key() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("操作系统随机数源不可用");
    let id: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("acp-conn-{id}")
}

fn busy(langs: &Languages) -> Response {
    api::error_response(
        StatusCode::TOO_MANY_REQUESTS,
        "busy",
        &l10n::text(langs, "gateway-agent-acp-busy", &[]),
    )
}

fn refused(langs: &Languages, id: &'static str, args: &[(&str, Arg<'_>)]) -> Response {
    api::error_response(
        StatusCode::FORBIDDEN,
        "permission_denied",
        &l10n::text(langs, id, args),
    )
}

fn unavailable(langs: &Languages, id: &'static str, args: &[(&str, Arg<'_>)]) -> Response {
    api::error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_stopped",
        &l10n::text(langs, id, args),
    )
}

fn is_websocket_upgrade(req: &Request) -> bool {
    req.method() == Method::GET
        && req
            .headers()
            .get(UPGRADE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}
