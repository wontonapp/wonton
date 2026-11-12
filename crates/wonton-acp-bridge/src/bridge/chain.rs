//! 桥的轮次登记（ADR-0061 第 10 节）：每轮 `session/prompt` 前向核心领链（`POST /agent/chains`，
//! 总线令牌在路径里、Bot 的 Matrix 令牌在头里），拿到链才发给 Agent。被拒（上限、已停等）或
//! 核心不可用时不发，房间里按类别说明。一轮结束（成功、取消、出错）报 `End`，报不上只记日志：
//! 核心重启时链会转成 `Ended`，不为此加重试。

use std::time::Duration;

use url::Url;
use wonton_l10n::Lang;
use wonton_protocol::{
    BOT_TOKEN_HEADER, ChainError, ChainLimitKind, ChainTurnGrant, ChainTurnReport,
};

use crate::bridge::Job;
use crate::error::{Detail, Error, format_error};
use crate::l10n::text;
use crate::room::Room;
use crate::topics::SessionKey;

/// `POST /agent/chains` 是几毫秒的账本操作；超时按普通接口对待，不加长。
const TIMEOUT: Duration = Duration::from_secs(15);

/// 领链失败：核心给了类别，还是根本没到核心。
pub(super) enum StartFailure {
    /// 核心应答了一个 [`ChainError`] 类别；`detail` 是核心的 `X-Wonton-Detail`（如有），
    /// `status` 是 HTTP 状态码，给不认识类别时的说明用。
    Refused {
        error: ChainError,
        detail: Option<String>,
        status: u16,
    },
    /// 连不上核心、超时或应答读不了。
    Unavailable(Error),
}

/// 调用链接口的客户端：一座桥一个，端点和总线令牌从 `WONTON_MCP_URL` 来。
#[derive(Clone)]
pub(crate) struct Client {
    http: reqwest::Client,
    endpoint: Url,
    matrix_token: String,
}

impl Client {
    pub(crate) fn new(mcp_url: &str, matrix_token: &str) -> Result<Self, Error> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .map_err(|error| {
                    Error::InvalidInput(Detail::msg_with(
                        "chain-client",
                        [("error", error.to_string())],
                    ))
                })?,
            endpoint: agent_endpoint(mcp_url, wonton_protocol::AGENT_CHAINS_PATH)?,
            matrix_token: matrix_token.to_owned(),
        })
    }

    /// 报一轮开始。响应是 [`ChainTurnGrant`]，或错误体 [`ChainError`]（类别在体里，
    /// 状态码只给不认识类别的调用方用，见核心 `ledger_error`）。
    async fn start(&self, report: ChainTurnReport) -> Result<ChainTurnGrant, StartFailure> {
        let unreachable = |error: String| {
            StartFailure::Unavailable(Error::TransportUnavailable(Detail::msg_with(
                "chain-unreachable",
                [("error", error)],
            )))
        };
        let response = self
            .http
            .post(self.endpoint.clone())
            .header(BOT_TOKEN_HEADER, &self.matrix_token)
            .json(&report)
            .send()
            .await
            .map_err(|error| unreachable(error.to_string()))?;
        let status = response.status();
        // 头先取：`bytes` 会消费响应。
        let detail = response
            .headers()
            .get("x-wonton-detail")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .bytes()
            .await
            .map_err(|error| unreachable(error.to_string()))?;
        if status.is_success() {
            return serde_json::from_slice::<ChainTurnGrant>(&body).map_err(|error| {
                StartFailure::Unavailable(Error::TransportUnavailable(Detail::msg_with(
                    "chain-invalid",
                    [
                        ("status", status.as_u16().to_string()),
                        ("error", error.to_string()),
                    ],
                )))
            });
        }
        match serde_json::from_slice::<ChainError>(&body) {
            Ok(error) => Err(StartFailure::Refused {
                error,
                detail,
                status: status.as_u16(),
            }),
            Err(_) => Err(StartFailure::Unavailable(Error::TransportUnavailable(
                Detail::data(format!("HTTP {status}")),
            ))),
        }
    }

    /// 报一轮结束。Best-effort：失败只记日志（核心重启时链转 `Ended`；报不上的会话
    /// 下一轮领链会被拒 `invalid_input`，房间里会如实说明）。
    pub(crate) async fn end(&self, session: &str) {
        let report = ChainTurnReport::End {
            session: session.to_owned(),
        };
        let result = self
            .http
            .post(self.endpoint.clone())
            .header(BOT_TOKEN_HEADER, &self.matrix_token)
            .json(&report)
            .send()
            .await;
        if let Err(error) = result {
            tracing::warn!(session, error = %error, "轮次结束没报上");
        }
    }
}

/// 总线地址 → Agent 接口的地址：`/mcp/<令牌>` 的令牌挪到给定路径后面，和确认卡片
/// 接口同一道门。没有令牌的插件地址（`/mcp`）照旧不带。
pub(crate) fn agent_endpoint(mcp_url: &str, path: &str) -> Result<Url, Error> {
    let mut endpoint = Url::parse(mcp_url).map_err(|error| {
        Error::InvalidInput(Detail::msg_with(
            "chain-url",
            [("error", error.to_string())],
        ))
    })?;
    let token = endpoint
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| *segment != "mcp")
        .map(str::to_owned);
    endpoint.set_path(&match token {
        Some(token) => format!("{path}/{token}"),
        None => path.to_owned(),
    });
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    Ok(endpoint)
}

/// 一轮开始前领链：拿到链才发 `session/prompt`。被拒（上限、已停等）或核心不可用时
/// 不发，在房间里按类别说明，返回 `false`。
pub(super) async fn begin<R: Room>(
    chains: &Client,
    room: &R,
    lang: Lang,
    key: &SessionKey,
    trigger: &Job,
    session_id: &str,
) -> bool {
    let report = ChainTurnReport::Start {
        session: session_id.to_owned(),
        room: room.room_id(),
        thread: match key {
            SessionKey::Main => None,
            SessionKey::Topic(root) => Some(root.clone()),
        },
        sender: trigger.sender.clone(),
        event_id: trigger.event_id.clone(),
    };
    match chains.start(report).await {
        Ok(_) => true,
        Err(StartFailure::Refused {
            error: ChainError::LimitReached { kind },
            ..
        }) => {
            say(room, key, &limit_text(kind, lang)).await;
            false
        }
        Err(StartFailure::Refused {
            error: ChainError::Stopped,
            ..
        }) => {
            say(room, key, &text(lang, "chain-stopped", &[])).await;
            false
        }
        Err(StartFailure::Refused { detail, status, .. }) => {
            let detail = detail.unwrap_or_else(|| format!("HTTP {status}"));
            let _ = tell(
                room,
                key,
                &format_error(&Error::ChainRefused(Detail::data(detail)), lang),
            )
            .await;
            false
        }
        Err(StartFailure::Unavailable(error)) => {
            let _ = tell(room, key, &format_error(&error, lang)).await;
            false
        }
    }
}

/// 到的是哪一种上限，房间里的说明就写哪一种；不认识的类别照实说“用到上限”。
fn limit_text(kind: ChainLimitKind, lang: Lang) -> String {
    let id = match kind {
        ChainLimitKind::Depth => "chain-limit-depth",
        ChainLimitKind::Rounds => "chain-limit-rounds",
        ChainLimitKind::Cost => "chain-limit-cost",
        ChainLimitKind::Other => "chain-limit-other",
    };
    text(lang, id, &[])
}

/// 上限、已停这类说明用 `m.notice`：主对话发一条，话题里发进话题。
async fn say<R: Room>(room: &R, key: &SessionKey, text: &str) {
    let _ = match key {
        SessionKey::Main => room.send_notice(text).await,
        SessionKey::Topic(root) => room.send_thread_notice(root, text).await,
    };
}

/// 其余类别的错误和传输错误走普通错误文字（ADR-0043 的类别前缀）。
async fn tell<R: Room>(room: &R, key: &SessionKey, text: &str) -> Result<(), Error> {
    match key {
        SessionKey::Main => room.send_text(text).await,
        SessionKey::Topic(root) => room.send_thread_text(root, text).await,
    }
}
