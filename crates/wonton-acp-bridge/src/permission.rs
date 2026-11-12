use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use serde_json::{Value, json};
use url::Url;
use wonton_l10n::{Arg, Lang};
use wonton_protocol::{
    AGENT_PERMISSION_PATH, AgentPermissionKind, AgentPermissionOption, AgentPermissionOutcome,
    AgentPermissionRequest, ConfirmationOrigin,
};

use crate::{
    error::{Detail, Error, Result},
    l10n::text,
};

const REQUEST_GRACE_SECS: u64 = 5;

#[derive(Clone, Debug)]
pub struct PendingPermission {
    pub request: AgentPermissionRequest,
    pub option_ids: HashMap<String, String>,
    /// 工具调用带的文件路径（ACP `locations[].path`），用来查和别的会话的冲突。
    pub locations: Vec<String>,
    /// 工具类别（ACP `toolCall.kind`）；查冲突只看 edit/delete/move。
    pub kind: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionResult {
    Selected {
        option_id: String,
        /// 拒绝时给 Agent 的理由，随应答的 `_meta.reason` 一起发。
        reason: Option<String>,
    },
    Cancelled,
    Expired,
}

impl PermissionResult {
    /// 只选了一项、没有附加理由。
    pub fn selected(option_id: String) -> Self {
        Self::Selected {
            option_id,
            reason: None,
        }
    }
}

#[derive(Clone)]
pub struct PermissionBroker {
    client: reqwest::Client,
    endpoint: Url,
}

impl PermissionBroker {
    pub fn new(mcp_url: &str) -> Result<Self> {
        let mut endpoint = Url::parse(mcp_url).map_err(|error| {
            Error::InvalidInput(Detail::msg_with(
                "perm-mcp-url",
                [("error", error.to_string())],
            ))
        })?;
        // 权限接口挂在同一个总线地址后面，路径里的总线令牌要带过去（`/mcp/<令牌>` → `/agent/permissions/<令牌>`，
        // 核心靠它认出是哪个 Bot）；没有令牌的插件地址（`/mcp`）照旧不带。
        let token = endpoint
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|segment| *segment != "mcp")
            .map(str::to_owned);
        endpoint.set_path(&match token {
            Some(token) => format!("{AGENT_PERMISSION_PATH}/{token}"),
            None => AGENT_PERMISSION_PATH.to_owned(),
        });
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(
                wonton_protocol::AGENT_PERMISSION_TTL_SECS + REQUEST_GRACE_SECS + 5,
            ))
            .build()
            .map_err(|error| {
                Error::InvalidInput(Detail::msg_with(
                    "perm-client",
                    [("error", error.to_string())],
                ))
            })?;
        Ok(Self { client, endpoint })
    }

    pub async fn ask(&self, pending: PendingPermission) -> Result<PermissionResult> {
        let response = tokio::time::timeout(
            Duration::from_secs(wonton_protocol::AGENT_PERMISSION_TTL_SECS + REQUEST_GRACE_SECS),
            async {
                let response = self
                    .client
                    .post(self.endpoint.clone())
                    .json(&pending.request)
                    .send()
                    .await
                    .map_err(|error| {
                        Error::ConfirmationUnavailable(Detail::msg_with(
                            "perm-broker-connect",
                            [("error", error.to_string())],
                        ))
                    })?;
                let status = response.status();
                let body = response.bytes().await.map_err(|error| {
                    Error::ConfirmationUnavailable(Detail::msg_with(
                        "perm-broker-read",
                        [("error", error.to_string())],
                    ))
                })?;
                Ok::<_, Error>((status, body))
            },
        )
        .await
        .map_err(|_| Error::ConfirmationUnavailable(Detail::msg("perm-broker-timeout")))??;
        let (status, body) = response;
        if !status.is_success() {
            let detail = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|value| value["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(match status.as_u16() {
                400 => Error::InvalidInput(Detail::data(detail)),
                429 => Error::Busy(Detail::data(detail)),
                _ => Error::ConfirmationUnavailable(Detail::data(detail)),
            });
        }
        let outcome: AgentPermissionOutcome = serde_json::from_slice(&body).map_err(|error| {
            Error::ConfirmationUnavailable(Detail::msg_with(
                "perm-broker-invalid",
                [("error", error.to_string())],
            ))
        })?;
        Ok(match outcome {
            AgentPermissionOutcome::Selected { choice_id } => pending
                .option_ids
                .get(&choice_id)
                .cloned()
                .map(PermissionResult::selected)
                .unwrap_or(PermissionResult::Cancelled),
            AgentPermissionOutcome::Cancelled => PermissionResult::Cancelled,
            AgentPermissionOutcome::Expired => PermissionResult::Expired,
        })
    }
}

pub fn parse_pending(
    params: &Value,
    lang: Lang,
    origin: Option<ConfirmationOrigin>,
) -> Result<PendingPermission> {
    let tool_call = &params["toolCall"];
    let tool_call_id = required_string(tool_call, "toolCallId")?;
    let title = required_string(tool_call, "title")?;
    let description = tool_call
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            tool_call["kind"]
                .as_str()
                .map(|kind| text(lang, "perm-tool-kind", &[("kind", Arg::from(kind))]))
        });
    let arguments = tool_call
        .get("rawInput")
        .filter(|value| !value.is_null())
        .map(|value| redact_secrets(value, lang));
    let mut locations: Vec<String> = tool_call["locations"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["path"].as_str().filter(|path| !path.is_empty()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    // 有的适配层不带 locations，只在参数里给文件路径（如 Claude 的 file_path）。
    if locations.is_empty() {
        locations = ["file_path", "path", "notebook_path"]
            .iter()
            .filter_map(|field| tool_call["rawInput"][field].as_str())
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect();
    }
    let mut options = Vec::new();
    let mut option_ids = HashMap::new();
    let mut seen_option_ids = HashSet::new();
    for (index, option) in params["options"]
        .as_array()
        .ok_or_else(|| Error::InvalidInput(Detail::msg("perm-missing-options")))?
        .iter()
        .enumerate()
    {
        let option_id = required_string(option, "optionId")?;
        if !seen_option_ids.insert(option_id.clone()) {
            return Err(Error::InvalidInput(Detail::msg("perm-duplicate-option-id")));
        }
        let kind: AgentPermissionKind = serde_json::from_value(json!(option["kind"].clone()))
            .map_err(|error| {
                Error::InvalidInput(Detail::msg_with(
                    "perm-invalid-kind",
                    [("error", error.to_string())],
                ))
            })?;
        let name = required_string(option, "name")?;
        let choice_id = format!("acp-{index}");
        option_ids.insert(choice_id.clone(), option_id);
        options.push(AgentPermissionOption {
            choice_id,
            kind,
            name,
        });
    }
    if options.is_empty() {
        return Err(Error::InvalidInput(Detail::msg("perm-empty-options")));
    }
    Ok(PendingPermission {
        request: AgentPermissionRequest {
            tool_call_id,
            title,
            description,
            arguments,
            options,
            // 会话的来源会话（ADR-0016），核心当不可信输入核对；没报时卡片落到 Bot 的主房间。
            origin,
            session: None,
        },
        option_ids,
        locations,
        kind: tool_call["kind"].as_str().map(str::to_owned),
    })
}

fn required_string(value: &Value, field: &str) -> Result<String> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::InvalidInput(Detail::msg_with(
                "perm-missing-field",
                [("field", field.to_owned())],
            ))
        })
}

fn redact_secrets(value: &Value, lang: Lang) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let lower = key.to_ascii_lowercase();
                    let redacted = lower.contains("token")
                        || lower.contains("secret")
                        || lower.contains("password")
                        || lower.contains("api_key")
                        || lower.contains("apikey")
                        || lower == "key";
                    (
                        key.clone(),
                        if redacted {
                            Value::String(text(lang, "perm-redacted", &[]))
                        } else {
                            redact_secrets(value, lang)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| redact_secrets(value, lang))
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l10n::zh;
    use std::net::SocketAddr;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
    };

    fn pending() -> PendingPermission {
        PendingPermission {
            request: AgentPermissionRequest {
                tool_call_id: "call-1".into(),
                title: "运行 ls".into(),
                description: None,
                arguments: None,
                options: vec![AgentPermissionOption {
                    choice_id: "acp-0".into(),
                    kind: AgentPermissionKind::AllowOnce,
                    name: "允许一次".into(),
                }],
                origin: None,
                session: None,
            },
            option_ids: HashMap::from([(String::from("acp-0"), String::from("allow_once"))]),
            locations: Vec::new(),
            kind: None,
        }
    }

    async fn permission_server(
        body: Value,
        status: &str,
    ) -> (SocketAddr, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).await.unwrap();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    content_length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut body_bytes = vec![0; content_length];
            tokio::io::AsyncReadExt::read_exact(&mut reader, &mut body_bytes)
                .await
                .unwrap();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.to_string().len(),
                body
            );
            reader
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
            String::from_utf8(body_bytes).unwrap()
        });
        (address, task)
    }

    async fn ask_from(body: Value, status: &str) -> (Result<PermissionResult>, String) {
        let (address, server) = permission_server(body, status).await;
        let broker = PermissionBroker::new(&format!("http://{address}/mcp")).unwrap();
        let result = broker.ask(pending()).await;
        (result, server.await.unwrap())
    }

    #[test]
    fn permission_url_keeps_the_bus_token() {
        let tokenized =
            PermissionBroker::new("http://mcp.wonton.internal:8080/mcp/abc123").unwrap();
        assert_eq!(
            tokenized.endpoint.as_str(),
            format!("http://mcp.wonton.internal:8080{AGENT_PERMISSION_PATH}/abc123")
        );
        let plain = PermissionBroker::new("http://mcp.wonton.internal:8080/mcp").unwrap();
        assert_eq!(plain.endpoint.path(), AGENT_PERMISSION_PATH);
    }

    #[test]
    fn maps_acp_options_to_opaque_core_choices() {
        let pending = parse_pending(
            &json!({
                "toolCall": {
                    "toolCallId": "call-1",
                    "title": "运行命令",
                    "kind": "execute",
                    "locations": [{"path": "src/a.rs"}],
                    "rawInput": {"command": "echo hi", "api_token": "secret"}
                },
                "options": [
                    {"optionId": "allow_once", "kind": "allow_once", "name": "允许一次"},
                    {"optionId": "reject_once", "kind": "reject_once", "name": "拒绝"}
                ]
            }),
            zh(),
            None,
        )
        .unwrap();
        assert_eq!(pending.request.options[0].choice_id, "acp-0");
        assert_eq!(pending.option_ids["acp-1"], "reject_once");
        assert_eq!(pending.locations, ["src/a.rs"]);
        assert_eq!(pending.kind.as_deref(), Some("execute"));
        assert_eq!(
            pending.request.arguments.as_ref().unwrap()["api_token"],
            "[已隐藏]"
        );
        // 没登记来源会话时不报，卡片落到 Bot 的主房间。
        let body = serde_json::to_value(&pending.request).unwrap();
        assert_eq!(body["tool_call_id"], "call-1");
        assert!(body.get("origin").is_none());
    }

    #[test]
    fn permission_request_carries_the_session_origin() {
        let pending = parse_pending(
            &json!({
                "toolCall": {"toolCallId": "call-1", "title": "改文件", "kind": "edit"},
                "options": [{"optionId": "allow_once", "kind": "allow_once", "name": "允许"}]
            }),
            zh(),
            Some(ConfirmationOrigin {
                room_id: "!room:example.org".into(),
                thread_id: Some("$root:example.org".into()),
            }),
        )
        .unwrap();
        assert_eq!(
            pending.request.origin,
            Some(ConfirmationOrigin {
                room_id: "!room:example.org".into(),
                thread_id: Some("$root:example.org".into()),
            })
        );
        let body = serde_json::to_value(&pending.request).unwrap();
        assert_eq!(body["origin"]["room_id"], "!room:example.org");
        assert_eq!(body["origin"]["thread_id"], "$root:example.org");
    }

    #[tokio::test]
    async fn selected_choice_maps_back_to_acp_option_id() {
        let (result, request) =
            ask_from(json!({"outcome":"selected","choice_id":"acp-0"}), "200 OK").await;
        assert_eq!(
            result.unwrap(),
            PermissionResult::Selected {
                option_id: "allow_once".into(),
                reason: None,
            }
        );
        assert!(request.contains("\"tool_call_id\":\"call-1\""));
    }

    #[tokio::test]
    async fn cancelled_and_expired_are_not_approval() {
        let (cancelled, _) = ask_from(json!({"outcome":"cancelled"}), "200 OK").await;
        let (expired, _) = ask_from(json!({"outcome":"expired"}), "200 OK").await;
        assert_eq!(cancelled.unwrap(), PermissionResult::Cancelled);
        assert_eq!(expired.unwrap(), PermissionResult::Expired);
    }

    #[tokio::test]
    async fn confirmation_errors_are_classified_without_approval() {
        let (invalid, _) = ask_from(
            json!({"error":{"message":"bad request"}}),
            "400 Bad Request",
        )
        .await;
        let (busy, _) = ask_from(
            json!({"error":{"message":"too many"}}),
            "429 Too Many Requests",
        )
        .await;
        let (unavailable, _) = ask_from(
            json!({"error":{"message":"down"}}),
            "503 Service Unavailable",
        )
        .await;
        assert!(
            matches!(invalid, Err(Error::InvalidInput(detail)) if detail.format(zh()) == "bad request")
        );
        assert!(matches!(busy, Err(Error::Busy(detail)) if detail.format(zh()) == "too many"));
        assert!(
            matches!(unavailable, Err(Error::ConfirmationUnavailable(detail)) if detail.format(zh()) == "down")
        );
    }
}
