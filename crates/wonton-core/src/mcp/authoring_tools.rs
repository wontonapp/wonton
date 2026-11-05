//! Agent 写插件用的只读工具（ADR-0033 的“Agent 写插件：从聊天到底栏”）：读说明和例子、校验 outbox 里
//! 的插件包、看插件状态。都不是敏感工具，不发卡片。安装（`install_plugin` 的 `dir`）和判定插件是谁写的
//! 在下一步。

use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};

use super::Bus;
use super::Caller;
use super::core_tools::tool;
use crate::files::share_fs::saving;
use crate::plugin::PluginId;
use crate::plugin::check::{self, Report};
use crate::plugin::store_info::SPEC_VERSION;

pub(super) const GUIDE: &str = "plugin_guide";
pub(super) const CHECK: &str = "check_plugin";
pub(super) const STATUS: &str = "plugin_status";

/// 说明和例子随核心走：这台核心支持的规范版本对应这一份，机器没外网也能用（wonton-docs 的
/// `skills/wonton-plugin/`，随规范一起改）。
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
            "读写插件用的说明：这台核心支持的规范版本、完整说明和两个最小例子的全部文件。主人想要新页面、\
             新应用或小工具，或者要改你之前写的插件时，先调这个。",
            json!({"type": "object", "properties": {}}),
            true,
        ),
        tool(
            CHECK,
            "校验你 outbox 里的一个插件目录，一次列出全部问题和提醒。不改任何东西，不装。改到没有 problems \
             为止，这一步不用问主人。",
            json!({"type": "object", "properties": {
                "dir": {"type": "string", "description": "outbox 里的相对路径，如 myapp"},
            }, "required": ["dir"]}),
            true,
        ),
        tool(
            STATUS,
            "看一个已装插件的状态：运行状态、健康，和它网页探测的状态码。你看不到页面长什么样，装完请主人看。",
            json!({"type": "object", "properties": {
                "id": {"type": "string", "description": "插件 ID，如 app.example.hello"},
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

pub(super) async fn check(bus: &Bus, caller: &Caller, args: &JsonObject) -> Result<Value, String> {
    let dir = args
        .get("dir")
        .and_then(Value::as_str)
        .filter(|dir| !dir.is_empty())
        .ok_or("invalid_input：缺少参数 dir")?;
    let app_data = bus.plugins.app_data_dir(&caller.id);
    let (opened, path) = saving::outbox_dir(&app_data, dir).map_err(|e| e.to_string())?;
    let before = inode_of(&opened, dir)?;
    // `check` 经路径读目录里的文件；读完核对路径还是打开时的那个目录，中间被换掉（比如换成软链接）就报错。
    let report = match check::check(&path, false, bus.plugins.images()).await {
        Ok(report) => report_value(&report),
        // 清单解析不了也是校验结果，不是工具错误。
        Err(e) => json!({ "problems": [e] }),
    };
    let after = saving::outbox_dir(&app_data, dir).map_err(|e| e.to_string())?;
    if inode_of(&after.0, dir)? != before {
        return Err(format!("invalid_input：{dir} 在校验时被改动，再试一次"));
    }
    Ok(report)
}

fn inode_of(fd: &std::os::fd::OwnedFd, what: &str) -> Result<(u64, u64), String> {
    let stat = rustix::fs::fstat(fd).map_err(|e| format!("invalid_input：{what} 读不了：{e}"))?;
    Ok((stat.st_dev, stat.st_ino))
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

pub(super) async fn status(bus: &Bus, args: &JsonObject) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("invalid_input：缺少参数 id")?;
    let id = PluginId::parse(id).map_err(|e| format!("invalid_input：{e}"))?;
    let view = bus
        .plugins
        .list()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("invalid_input：没有装 {id}"))?;
    let mut result = json!({
        "id": id.to_string(),
        "status": view.status,
        "url": view.url,
    });
    if let Some(health) = view.health {
        result["health"] = json!(health);
    }
    match bus.plugins.webui_status(&id).await {
        Some(Ok(code)) => result["web_status"] = json!(code),
        // 连不上、说不出状态码时如实报，不伪装成探测成功。
        Some(Err(e)) => result["web_error"] = json!(e.to_string()),
        None => {}
    }
    Ok(result)
}
