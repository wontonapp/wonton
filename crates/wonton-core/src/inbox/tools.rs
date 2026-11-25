//! 核心在 MCP 总线上给 Agent 的收件箱工具（ADR-0119 第 3 节）：ask/present/recap/confirm/notify/wait/cancel/update。
//!
//! 工具只做参数校验和建条目；页面在发送时读进快照、按声明的域名生成 CSP。核心不认识具体 Agent。

use std::time::Duration;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool, ToolAnnotations};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use wonton_protocol::inbox_request::{
    InboxAskRequest, InboxCancelRequest, InboxConfirmRequest, InboxNoticeRequest,
    InboxPresentRequest, InboxRecapRequest, InboxUpdateRequest, InboxWaitRequest,
};
use wonton_protocol::inbox_validate::{validate_choice, validate_network_domains};
use wonton_protocol::{
    ChoicePayload, ConfirmPayload, INBOX_ASK_TOOL, INBOX_CANCEL_TOOL, INBOX_CONFIRM_TOOL,
    INBOX_NOTICE_TOOL, INBOX_PRESENT_TOOL, INBOX_RECAP_TOOL, INBOX_RESOURCE_SCHEME,
    INBOX_UPDATE_TOOL, INBOX_WAIT_TOOL, InboxAnswer, InboxAudience, InboxCreateReply as Reply,
    InboxItem, InboxLevel, InboxPayload, InboxQuestion, InboxRecipient, InboxSender,
    InboxSenderKind, InboxStatus, NoticePayload, PagePayload, RecapPayload,
};

use super::page;
use super::store::{CreateInput, Inbox, InboxError, NewPage};
use crate::mcp::audience::Subject;
use crate::mcp::{Bus, Caller, ToolFailure};
use crate::plugin::manifest::Permission;

fn tool(name: &str, description: &str, input: Value) -> Tool {
    Tool::new(name.to_owned(), description.to_owned(), schema(input)).with_annotations(
        ToolAnnotations::new()
            .read_only(false)
            .destructive(false)
            .open_world(false),
    )
}

fn schema(value: Value) -> std::sync::Arc<JsonObject> {
    match value {
        Value::Object(map) => std::sync::Arc::new(map),
        _ => unreachable!("工具参数是 JSON 对象"),
    }
}

pub(crate) fn definitions() -> Vec<Tool> {
    vec![
        tool(
            INBOX_ASK_TOOL,
            "给主人发一张问题卡，答案按题目结构化回来。1–4 题、每题 2–4 个选项（或 0 个自由回答），\
             永远有“其他”。阻塞时一直等到主人回答或过期；异步时立刻返回 item_id，之后用 wonton__inbox_wait 等。\
             一件事一张卡，简单的事在聊天里问。",
            json!({"type": "object", "properties": {
                "title": {"type": "string"},
                "questions": {"type": "array", "items": {"type": "object"}},
                "blocking": {"type": "boolean", "default": true},
                "expires_in_ms": {"type": "integer"},
                "dedupe_key": {"type": "string"},
                "audience": {"type": "string", "enum": ["keeper", "admins"]}
            }, "required": ["questions"]}),
        ),
        tool(
            INBOX_PRESENT_TOOL,
            "把发送方自己的一个 `ui://` MCP 资源读成快照，作为可翻看的页面卡片发进收件箱。页面默认断网；\
             要联网就在 network_domains 里列域名（https:// 或 wss://），卡上会写“会访问”。",
            json!({"type": "object", "properties": {
                "title": {"type": "string"},
                "resource_uri": {"type": "string"},
                "caption": {"type": "string"},
                "network_domains": {"type": "array", "items": {"type": "string"}},
                "actions": {"type": "array", "items": {"type": "object"}},
                "blocking": {"type": "boolean", "default": false},
                "expires_in_ms": {"type": "integer"},
                "dedupe_key": {"type": "string"},
                "audience": {"type": "string", "enum": ["keeper", "admins"]}
            }, "required": ["resource_uri", "caption"]}),
        ),
        tool(
            INBOX_RECAP_TOOL,
            "发一条两三句的收尾：现在是什么状态、还缺什么，至多一个问题、1–2 个按钮。",
            json!({"type": "object", "properties": {
                "summary": {"type": "string"},
                "question": {"type": "string"},
                "actions": {"type": "array", "items": {"type": "object"}},
                "blocking": {"type": "boolean", "default": false},
                "expires_in_ms": {"type": "integer"},
                "dedupe_key": {"type": "string"},
                "audience": {"type": "string", "enum": ["keeper", "admins"]}
            }, "required": ["summary"]}),
        ),
        tool(
            INBOX_CONFIRM_TOOL,
            "发一张固定确认：允许/拒绝（发送方支持时加本会话/总是）。高风险操作把原始命令放进 detail，默认折叠。",
            json!({"type": "object", "properties": {
                "title": {"type": "string"},
                "body": {"type": "string"},
                "detail": {"type": "string"},
                "risk": {"type": "string", "enum": ["high", "very_high", "highest"]},
                "choices": {"type": "array", "items": {"type": "object"}},
                "expires_in_ms": {"type": "integer"},
                "dedupe_key": {"type": "string"},
                "audience": {"type": "string", "enum": ["keeper", "admins"]}
            }, "required": ["title", "body"]}),
        ),
        tool(
            INBOX_NOTICE_TOOL,
            "发一条只读通知，不进角标。只是告知用 notice；要人拍板用 ask/confirm。",
            json!({"type": "object", "properties": {
                "title": {"type": "string"},
                "body": {"type": "string"},
                "level": {"type": "string", "enum": ["info", "warning", "critical"]},
                "link": {"type": "string"},
                "action": {"type": "object"},
                "dedupe_key": {"type": "string"},
                "audience": {"type": "string", "enum": ["keeper", "admins"]}
            }, "required": ["title", "body", "level"]}),
        ),
        tool(
            INBOX_WAIT_TOOL,
            "等一条条目的答案或状态变化；不写 after_revision 就看当前。异步条目建了之后用它等结果。",
            json!({"type": "object", "properties": {
                "item_id": {"type": "string"},
                "after_revision": {"type": "integer"}
            }, "required": ["item_id"]}),
        ),
        tool(
            INBOX_CANCEL_TOOL,
            "发送方撤回一条还没答的条目；阻塞着的调用方会拿到“已撤回”。",
            json!({"type": "object", "properties": {
                "item_id": {"type": "string"},
                "reason": {"type": "string"}
            }, "required": ["item_id"]}),
        ),
        tool(
            INBOX_UPDATE_TOOL,
            "更新一条还没答的条目的摘要/进度（长任务），不新建、不刷屏。",
            json!({"type": "object", "properties": {
                "item_id": {"type": "string"},
                "summary": {"type": "string"},
                "progress": {"type": "number"}
            }, "required": ["item_id"]}),
        ),
    ]
}

pub(crate) async fn call(
    bus: &Bus,
    caller: &Caller,
    tool: &str,
    args: JsonObject,
) -> Result<CallToolResult, ToolFailure> {
    let Some(inbox) = bus.inbox() else {
        return Err(ToolFailure::new("service_unavailable", "收件箱没开"));
    };
    let value = Value::Object(args);
    match tool {
        "inbox_ask" => ask(bus, &inbox, caller, value).await,
        "inbox_present" => present(bus, &inbox, caller, value).await,
        "inbox_recap" => recap(bus, &inbox, caller, value),
        "inbox_confirm" => confirm(bus, &inbox, caller, value),
        "inbox_notify" => notify(bus, &inbox, caller, value),
        "inbox_wait" => wait(&inbox, caller, &value).await,
        "inbox_cancel" => cancel(&inbox, caller, value),
        "inbox_update" => update(&inbox, caller, value),
        other => Err(ToolFailure::new(
            "invalid_input",
            format!("没有工具 {other}"),
        )),
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, ToolFailure> {
    serde_json::from_value(value)
        .map_err(|e| ToolFailure::new("invalid_input", format!("参数不对：{e}")))
}

fn failure(error: InboxError) -> ToolFailure {
    ToolFailure::new(error.kind(), error.to_string())
}

fn sender(caller: &Caller) -> InboxSender {
    InboxSender {
        kind: if caller.bot.is_some() {
            InboxSenderKind::Agent
        } else {
            InboxSenderKind::Plugin
        },
        id: caller.id.to_string(),
        display_name: caller.name.clone(),
        avatar: None,
    }
}

fn recipient(
    bus: &Bus,
    caller: &Caller,
    audience: InboxAudience,
) -> Result<InboxRecipient, ToolFailure> {
    if audience == InboxAudience::Admins {
        let core = caller.id.as_str() == "app.wonton.core";
        if !core
            && !bus
                .plugins()
                .has_permission(&caller.id, &Permission::Notify)
        {
            return Err(ToolFailure::new(
                "permission_denied",
                "发所有管理员要 notify 权限",
            ));
        }
        return Ok(InboxRecipient::Admins);
    }
    let username = bus.recipient(caller, Subject::Tool).unwrap_or_else(|| {
        // 确认表的约定：None 是主人；共享 Bot 和没有 Bot 的插件也落到主人。
        bus.plugins().owner_name().to_owned()
    });
    Ok(InboxRecipient::Person { username })
}

fn payload_choice(request: &InboxAskRequest) -> Result<ChoicePayload, ToolFailure> {
    let payload = ChoicePayload {
        questions: request.questions.clone(),
    };
    validate_choice(&payload).map_err(|e| ToolFailure::new("invalid_input", e.to_string()))?;
    Ok(payload)
}

async fn ask(
    bus: &Bus,
    inbox: &std::sync::Arc<Inbox>,
    caller: &Caller,
    value: Value,
) -> Result<CallToolResult, ToolFailure> {
    if bus.inbox().is_none() {
        return Err(ToolFailure::new("service_unavailable", "收件箱没开"));
    }
    let request: InboxAskRequest = parse(value)?;
    let payload = payload_choice(&request)?;
    let created = inbox
        .create(CreateInput {
            kind: wonton_protocol::InboxType::Choice,
            sender: sender(caller),
            recipient: recipient(bus, caller, request.audience)?,
            title: request.title.unwrap_or_default(),
            body: String::new(),
            level: None,
            source: None,
            blocking: request.blocking,
            dedupe_key: request.dedupe_key,
            payload: InboxPayload::Choice(payload),
            expires_in_ms: request.expires_in_ms,
            page: None,
        })
        .map_err(failure)?;
    finish(inbox, created, request.blocking).await
}

async fn present(
    bus: &Bus,
    inbox: &std::sync::Arc<Inbox>,
    caller: &Caller,
    value: Value,
) -> Result<CallToolResult, ToolFailure> {
    let request: InboxPresentRequest = parse(value)?;
    let uri = request
        .resource_uri
        .as_deref()
        .ok_or_else(|| ToolFailure::new("invalid_input", "这个 PR 只收 resource_uri"))?;
    if !uri.starts_with(INBOX_RESOURCE_SCHEME) {
        return Err(ToolFailure::new("invalid_input", "资源要 ui:// 开头"));
    }
    validate_network_domains(&request.network_domains)
        .map_err(|e| ToolFailure::new("invalid_input", e.to_string()))?;
    let domains = request
        .network_domains
        .iter()
        .map(|d| wonton_protocol::normalize_network_domain(d).unwrap_or_else(|_| d.clone()))
        .collect::<Vec<_>>();
    let (html, mime) = fetch_resource(bus, caller, uri).await?;
    if !page::is_html(mime.as_deref()) {
        return Err(ToolFailure::new(
            "invalid_input",
            format!("资源 {uri} 不是 text/html"),
        ));
    }
    page::validate_size(html.len()).map_err(failure)?;
    let payload = PagePayload {
        resource_uri: Some(uri.to_owned()),
        file_id: None,
        caption: request.caption,
        network_domains: request.network_domains.clone(),
        actions: request.actions,
    };
    let created = inbox
        .create(CreateInput {
            kind: wonton_protocol::InboxType::Page,
            sender: sender(caller),
            recipient: recipient(bus, caller, request.audience)?,
            title: request.title.unwrap_or_else(|| payload.caption.clone()),
            body: String::new(),
            level: None,
            source: None,
            blocking: request.blocking,
            dedupe_key: request.dedupe_key,
            payload: InboxPayload::Page(payload),
            expires_in_ms: request.expires_in_ms,
            page: Some(NewPage {
                html,
                csp: page::policy(&domains),
            }),
        })
        .map_err(failure)?;
    finish(inbox, created, request.blocking).await
}

async fn fetch_resource(
    bus: &Bus,
    caller: &Caller,
    uri: &str,
) -> Result<(Vec<u8>, Option<String>), ToolFailure> {
    let Some(server) = bus
        .plugins()
        .mcp_servers()
        .into_iter()
        .find(|server| server.id == caller.id)
    else {
        return Err(ToolFailure::new(
            "service_unavailable",
            format!("{} 没有在运行的 MCP 服务", caller.id),
        ));
    };
    let result = bus
        .upstreams()
        .read_contents(&server, uri)
        .await
        .map_err(|e| ToolFailure::new("service_unavailable", e.to_string()))?;
    let Some(contents) = result.contents.into_iter().next() else {
        return Err(ToolFailure::new("invalid_input", "资源是空的"));
    };
    match contents {
        rmcp::model::ResourceContents::TextResourceContents {
            mime_type, text, ..
        } => Ok((text.into_bytes(), mime_type)),
        rmcp::model::ResourceContents::BlobResourceContents {
            mime_type, blob, ..
        } => {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(blob)
                .map_err(|e| {
                    ToolFailure::new("invalid_input", format!("资源不是合法 base64：{e}"))
                })?;
            Ok((bytes, mime_type))
        }
        other => Err(ToolFailure::new(
            "invalid_input",
            format!("读不懂的资源类型：{other:?}"),
        )),
    }
}

fn recap(
    bus: &Bus,
    inbox: &std::sync::Arc<Inbox>,
    caller: &Caller,
    value: Value,
) -> Result<CallToolResult, ToolFailure> {
    let request: InboxRecapRequest = parse(value)?;
    let payload = RecapPayload {
        summary: request.summary.clone(),
        question: request.question.clone(),
        actions: request.actions,
    };
    let created = inbox
        .create(CreateInput {
            kind: wonton_protocol::InboxType::Recap,
            sender: sender(caller),
            recipient: recipient(bus, caller, request.audience)?,
            title: request.summary.chars().take(120).collect(),
            body: request.summary,
            level: None,
            source: None,
            blocking: request.blocking,
            dedupe_key: request.dedupe_key,
            payload: InboxPayload::Recap(payload),
            expires_in_ms: request.expires_in_ms,
            page: None,
        })
        .map_err(failure)?;
    // 同步建条目；阻塞时由 HTTP 客户端随后用 wait 轮询，不在这里挂住命令。
    Ok(created_reply(created))
}

fn confirm(
    bus: &Bus,
    inbox: &std::sync::Arc<Inbox>,
    caller: &Caller,
    value: Value,
) -> Result<CallToolResult, ToolFailure> {
    let request: InboxConfirmRequest = parse(value)?;
    let payload = ConfirmPayload {
        title: request.title.clone(),
        body: request.body.clone(),
        detail: request.detail,
        risk: request.risk,
        choices: request.choices,
    };
    let created = inbox
        .create(CreateInput {
            kind: wonton_protocol::InboxType::Confirm,
            sender: sender(caller),
            recipient: recipient(bus, caller, request.audience)?,
            title: request.title,
            body: request.body,
            level: None,
            source: None,
            blocking: request.blocking,
            dedupe_key: request.dedupe_key,
            payload: InboxPayload::Confirm(payload),
            expires_in_ms: request.expires_in_ms,
            page: None,
        })
        .map_err(failure)?;
    Ok(created_reply(created))
}

fn notify(
    bus: &Bus,
    inbox: &std::sync::Arc<Inbox>,
    caller: &Caller,
    value: Value,
) -> Result<CallToolResult, ToolFailure> {
    let request: InboxNoticeRequest = parse(value)?;
    let payload = NoticePayload {
        title: request.title.clone(),
        body: request.body.clone(),
        level: request.level,
        link: request.link,
        action: request.action,
    };
    let created = inbox
        .create(CreateInput {
            kind: wonton_protocol::InboxType::Notice,
            sender: sender(caller),
            recipient: recipient(bus, caller, request.audience)?,
            title: request.title,
            body: request.body,
            level: Some(request.level),
            source: None,
            blocking: false,
            dedupe_key: request.dedupe_key,
            payload: InboxPayload::Notice(payload),
            expires_in_ms: None,
            page: None,
        })
        .map_err(failure)?;
    Ok(created_reply(created))
}

async fn wait(
    inbox: &Inbox,
    caller: &Caller,
    value: &Value,
) -> Result<CallToolResult, ToolFailure> {
    let request: InboxWaitRequest = parse(value.clone())?;
    let item = inbox
        .wait_for_sender(caller.id.as_str(), &request.item_id, request.after_revision)
        .await
        .map_err(failure)?;
    Ok(success_json(&json!({
        "item_id": item.id,
        "status": item.status,
        "answer": item.answer,
    })))
}

fn cancel(inbox: &Inbox, caller: &Caller, value: Value) -> Result<CallToolResult, ToolFailure> {
    let request: InboxCancelRequest = parse(value)?;
    let item = inbox
        .cancel_by_sender(caller.id.as_str(), &request.item_id, request.reason)
        .map_err(failure)?;
    Ok(ok_item(item))
}

fn update(inbox: &Inbox, caller: &Caller, value: Value) -> Result<CallToolResult, ToolFailure> {
    let request: InboxUpdateRequest = parse(value)?;
    let item = inbox
        .update_by_sender(
            caller.id.as_str(),
            &request.item_id,
            request.summary,
            request.progress,
        )
        .map_err(failure)?;
    Ok(ok_item(item))
}

async fn finish(
    inbox: &std::sync::Arc<Inbox>,
    created: super::store::CreateReply,
    blocking: bool,
) -> Result<CallToolResult, ToolFailure> {
    let item = created.item;
    let mut reply = Reply {
        item_id: item.id.clone(),
        status: item.status,
        warning: created.warning,
        answer: None,
    };
    if blocking && let Some(receiver) = created.waiter {
        let expires = item
            .expires_at_ms
            .unwrap_or_else(|| item.created_at_ms.saturating_add(600_000));
        let now = inbox.clock_now();
        let wait_ms = expires.saturating_sub(now).max(1);
        match tokio::time::timeout(Duration::from_millis(wait_ms), receiver).await {
            Ok(Ok(answer)) => {
                if let Some(status) = inbox.status(&item.id) {
                    reply.status = status;
                }
                reply.answer = Some(answer);
            }
            Ok(Err(_)) => reply.status = InboxStatus::Expired,
            Err(_) => reply.status = InboxStatus::Expired,
        }
    }
    Ok(success_json(&reply))
}

fn created_reply(created: super::store::CreateReply) -> CallToolResult {
    success_json(&Reply {
        item_id: created.item.id,
        status: created.item.status,
        warning: created.warning,
        answer: None,
    })
}

fn ok_item(item: InboxItem) -> CallToolResult {
    success_json(&item)
}

fn success_json<T: serde::Serialize>(value: &T) -> CallToolResult {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned());
    CallToolResult::success(vec![ContentBlock::text(text)])
}

/// `inbox_wait` 等答案时 `after_revision` 只是上次看到的版本；这里给测试和调用方保留一个未用的入口。
#[allow(dead_code)]
fn status_of(item: &InboxItem) -> InboxStatus {
    item.status
}

#[allow(dead_code)]
fn unused(_: InboxItem, _: InboxAnswer, _: InboxLevel, _: InboxQuestion, _: oneshot::Sender<()>) {}
