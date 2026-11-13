use std::sync::Arc;

use rmcp::model::{CallToolResult, JsonObject, Tool};
use serde_json::{Value, json};
use wonton_protocol::{HandOffRequest, HandOffStatusRequest};

use super::{Bus, Caller, ToolFailure, core_tools, handoff};

pub(super) const HAND_OFF: &str = "hand_off";
pub(super) const STATUS: &str = "hand_off_status";

pub(super) fn definitions() -> Vec<Tool> {
    vec![
        core_tools::tool(
            HAND_OFF,
            "把任务交给本服务器的另一个 Bot。开好会话并发出第一轮后立即返回 session 和 chain，不等它做完。\
             目标会在原房间、原话题里自己说进度和结果；不要再发消息点它的名，以免做两遍。\
             核心不会把结果交回给你；用 wonton__hand_off_status 查询，或等它叫你的名字。\
             目标归属人可能需要先在 App 确认。相同请求 ID 重试会复用；相同目标、相同原文且还在做时也复用。",
            json!({"type":"object","properties":{
                "target_bot":{"type":"object","properties":{
                    "plugin_id":{"type":"string"},"bot":{"type":"string"}
                },"required":["plugin_id","bot"],"additionalProperties":false},
                "message":{"type":"string","minLength":1}
            },"required":["target_bot","message"],"additionalProperties":false}),
            false,
        ),
        core_tools::tool(
            STATUS,
            "只读查询自己交出去的 Bot 会话，不叫醒目标、不计轮数。不填 session 时列出自己从当前房间、话题交出去的全部。\
             summary 是目标最近回复的末尾，最多 2000 字；不是目标已经完成任务的保证。",
            json!({"type":"object","properties":{"session":{"type":"string","minLength":1}},"additionalProperties":false}),
            true,
        ),
    ]
}

pub(super) async fn call(
    bus: &Arc<Bus>,
    caller: &Caller,
    tool: &str,
    args: JsonObject,
    activity: crate::bot_activity::Guard,
) -> CallToolResult {
    let name = super::naming::join(super::naming::CORE, tool);
    let value = Value::Object(args);
    let result = if !bus.plugins.declares_agent_acp(&caller.id) {
        Err(ToolFailure::new(
            "permission_denied",
            "调用方插件没有声明 agent_acp",
        ))
    } else if tool == STATUS {
        status(bus, caller, value.clone())
    } else {
        drop(activity);
        match parse(value.clone()) {
            Ok(request) => handoff::dispatch(bus, caller, request)
                .await
                .and_then(|opened| {
                    serde_json::to_value(opened)
                        .map_err(|error| ToolFailure::new("internal_error", error.to_string()))
                }),
            Err(error) => Err(error),
        }
    };
    match result {
        Ok(result) => {
            bus.audit.record(&super::audit::Record {
                caller: caller.id.as_str(),
                tool: &name,
                arguments: &value,
                decision: super::audit::Decision::Allowed,
                basis: None,
                outcome: Some("ok"),
                files: &[],
            });
            core_tools::success(&result)
        }
        Err(error) => bus.deny(caller, &name, &value, &error),
    }
}

fn parse(value: Value) -> Result<HandOffRequest, ToolFailure> {
    if value.as_object().is_none_or(|args| {
        args.keys()
            .any(|key| key != "target_bot" && key != "message")
    }) {
        return Err(ToolFailure::new(
            "invalid_input",
            "交接参数只接受 target_bot 和 message",
        ));
    }
    if value
        .get("target_bot")
        .and_then(Value::as_object)
        .is_none_or(|bot| bot.keys().any(|key| key != "plugin_id" && key != "bot"))
    {
        return Err(ToolFailure::new(
            "invalid_input",
            "target_bot 只接受 plugin_id 和 bot",
        ));
    }
    let request: HandOffRequest = serde_json::from_value(value)
        .map_err(|error| ToolFailure::new("invalid_input", error.to_string()))?;
    if request.message.trim().is_empty() {
        return Err(ToolFailure::new("invalid_input", "交代的话不能为空"));
    }
    crate::plugin::PluginId::parse(&request.target_bot.plugin_id)
        .map_err(|error| ToolFailure::new("invalid_input", error.to_string()))?;
    crate::plugin::bots::check_bot_name(&request.target_bot.bot)
        .map_err(|error| ToolFailure::new("invalid_input", error))?;
    Ok(request)
}

fn status(bus: &Bus, caller: &Caller, value: Value) -> Result<Value, ToolFailure> {
    if value
        .as_object()
        .is_none_or(|args| args.keys().any(|key| key != "session"))
    {
        return Err(ToolFailure::new("invalid_input", "查询参数只接受 session"));
    }
    let request: HandOffStatusRequest = serde_json::from_value(value)
        .map_err(|error| ToolFailure::new("invalid_input", error.to_string()))?;
    if request.session.as_deref() == Some("") {
        return Err(ToolFailure::new("invalid_input", "session 不能为空"));
    }
    let origin = handoff::conversation(bus, caller)?;
    let sessions = bus
        .handoffs
        .list(bus, caller, request.session.as_deref(), &origin)?;
    serde_json::to_value(sessions)
        .map_err(|error| ToolFailure::new("internal_error", error.to_string()))
}
