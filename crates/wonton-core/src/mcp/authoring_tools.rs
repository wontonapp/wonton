//! Agent 写插件用的只读工具（ADR-0033 的“Agent 写插件：从聊天到底栏”）：读说明和例子、校验 outbox 里
//! 的插件包、看插件状态。都不是敏感工具，不发卡片。安装（`install_plugin` 的 `dir`）和判定插件是谁写的
//! 在下一步。

use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};

use std::os::fd::AsFd;

use super::Bus;
use super::Caller;
use super::ToolFailure;
use super::core_tools::tool;
use crate::files::share_fs::saving;
use crate::plugin::PluginId;
use crate::plugin::check::{self, Report};
use crate::plugin::store_info::SPEC_VERSION;

pub(super) const GUIDE: &str = "plugin_guide";
pub(super) const CHECK: &str = "check_plugin";
pub(super) const STATUS: &str = "plugin_status";

/// 说明和例子随核心走：这台核心支持的规范版本对应这一份，机器没外网也能用。这份是源，
/// wonton-docs 的 `skills/wonton-plugin/` 与它保持一致，只保留指向那边文档的链接差异。
const GUIDE_TEXT: &str = include_str!("../../plugin-guide/SKILL.md");
const EXAMPLES: &[(&str, &[(&str, &str)])] = &[
    (
        "assets/static",
        &[
            (
                "docker-compose.yml",
                include_str!("../../plugin-guide/assets/static/docker-compose.yml"),
            ),
            (
                "index.html",
                include_str!("../../plugin-guide/assets/static/index.html"),
            ),
            (
                "tabs/home.svg",
                include_str!("../../plugin-guide/assets/static/tabs/home.svg"),
            ),
        ],
    ),
    (
        "assets/backend",
        &[
            (
                "docker-compose.yml",
                include_str!("../../plugin-guide/assets/backend/docker-compose.yml"),
            ),
            (
                "index.html",
                include_str!("../../plugin-guide/assets/backend/index.html"),
            ),
            (
                "server.py",
                include_str!("../../plugin-guide/assets/backend/server.py"),
            ),
            (
                "tabs/home.svg",
                include_str!("../../plugin-guide/assets/backend/tabs/home.svg"),
            ),
        ],
    ),
];

pub(super) fn definitions() -> Vec<Tool> {
    vec![
        tool(
            GUIDE,
            "The plugin-writing guide: the spec version this home server supports, the full guide, and every file of \
             two minimal examples. Call this first when the owner wants a new page, app, or small tool, or when you \
             need to change a plugin you wrote before.",
            json!({"type": "object", "properties": {}}),
            true,
        ),
        tool(
            CHECK,
            "Validate a plugin directory in your outbox and list all problems and warnings at once. Changes nothing, \
             installs nothing. Iterate until there are no problems; this step doesn't need the owner's approval.",
            json!({"type": "object", "properties": {
                "dir": {"type": "string", "description": "relative path in your outbox, e.g. myapp"},
            }, "required": ["dir"]}),
            true,
        ),
        tool(
            STATUS,
            "Check an installed plugin: run state, health, and the status code of its web probe. You can't see what \
             the page looks like; ask the owner to look after installing.",
            json!({"type": "object", "properties": {
                "id": {"type": "string", "description": "plugin ID, e.g. app.example.hello"},
            }, "required": ["id"]}),
            true,
        ),
    ]
}

pub(super) fn guide() -> Value {
    let examples: Value = EXAMPLES
        .iter()
        .map(|(name, files)| {
            let files: Value = files
                .iter()
                .map(|(path, content)| json!({"path": path, "content": content}))
                .collect();
            (name.to_string(), files)
        })
        .collect();
    json!({
        "spec_version": SPEC_VERSION.to_string(),
        "guide": GUIDE_TEXT,
        "examples": examples,
    })
}

pub(super) async fn check(
    bus: &Bus,
    caller: &Caller,
    args: &JsonObject,
) -> Result<Value, ToolFailure> {
    let dir = args
        .get("dir")
        .and_then(Value::as_str)
        .filter(|dir| !dir.is_empty())
        .ok_or_else(|| ToolFailure::new("invalid_input", "invalid_input: missing argument dir"))?;
    let app_data = bus.plugins.app_data_dir(&caller.id);
    // 目录 fd 一路钉住，包里的文件都从它读，中途换成软链接也出不去。
    let (opened, _) = saving::outbox_dir(&app_data, dir)
        .map_err(|e| ToolFailure::new(e.kind.as_str(), e.to_string()))?;
    let report = match check::check_at(opened.as_fd(), false, bus.plugins.images()).await {
        Ok(report) => report_value(&report),
        // 清单解析不了也是校验结果，不是工具错误。
        Err(e) => json!({ "problems": [e] }),
    };
    Ok(report)
}

fn report_value(report: &Report) -> Value {
    let files: Vec<Value> = report
        .files
        .iter()
        .map(|(path, bytes)| json!({"path": path, "bytes": bytes}))
        .collect();
    json!({
        "id": report.id,
        "version": report.version,
        "digest": report.digest,
        "files": files,
        "problems": report.problems,
        "warnings": report.warnings,
    })
}

pub(super) async fn status(
    bus: &Bus,
    caller: &Caller,
    args: &JsonObject,
) -> Result<Value, ToolFailure> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ToolFailure::new("invalid_input", "invalid_input: missing argument id"))?;
    let id = PluginId::parse(id)
        .map_err(|e| ToolFailure::new("invalid_input", format!("invalid_input: {e}")))?;
    let view = bus
        .plugins
        .list()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| {
            ToolFailure::new(
                "not_installed",
                format!("invalid_input: {id} is not installed"),
            )
        })?;
    let mut result = json!({
        "id": id.to_string(),
        "status": view.status,
        "url": view.url,
    });
    if let Some(health) = view.health {
        result["health"] = json!(health);
    }
    // 写这个插件的 Bot 看得到每个服务最近的日志；别的 Bot 和不是 Agent 写的插件不带（ADR-0033）。
    let writer = bus.plugins.writer(&id).ok().flatten();
    if writer.is_some_and(|w| w.plugin_id == caller.id.to_string()) {
        match bus.plugins.service_logs(&id).await {
            Ok(logs) if !logs.is_empty() => {
                result["logs"] = json!(
                    logs.into_iter()
                        .map(|(service, text)| json!({"service": service, "text": text}))
                        .collect::<Vec<_>>()
                );
            }
            Ok(_) => {}
            Err(e) => result["logs_error"] = json!(e.to_string()),
        }
    }
    match bus.plugins.webui_status(&id).await {
        Some(Ok(code)) => result["web_status"] = json!(code),
        // 连不上、说不出状态码时如实报，不伪装成探测成功。
        Some(Err(e)) => result["web_error"] = json!(e.to_string()),
        None => {}
    }
    Ok(result)
}
