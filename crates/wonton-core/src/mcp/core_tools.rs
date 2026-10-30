//! 核心自己的工具（插件规范的“工具与 MCP 总线”）：列出插件、在插件目录里找插件、安装和卸载。安装和卸载要主人
//! 确认。Bot 只能装目录里的插件，申请高权限的插件只能由主人在 App 里装（ADR-0016）。

use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool, ToolAnnotations};
use serde_json::{Value, json};

use super::audit::{Decision, Record};
use super::{Bus, Caller, Outcome, cards, naming};
use crate::plugin::render::dry_run;
use crate::plugin::{InstallError, PluginId, PluginPackage};

const LIST: &str = "list_plugins";
const SEARCH: &str = "search_catalog";
const INSTALL: &str = "install_plugin";
const UNINSTALL: &str = "uninstall_plugin";

fn schema(value: Value) -> Arc<JsonObject> {
    match value {
        Value::Object(map) => Arc::new(map),
        _ => unreachable!("工具的参数表是 JSON 对象"),
    }
}

fn tool(name: &str, description: &str, input: Value, read_only: bool) -> Tool {
    Tool::new(
        naming::join(naming::CORE, name),
        description.to_owned(),
        schema(input),
    )
    .with_annotations(
        ToolAnnotations::new()
            .read_only(read_only)
            .destructive(name == UNINSTALL)
            .open_world(false),
    )
}

pub fn definitions() -> Vec<Tool> {
    let id = json!({"type": "string", "description": "插件 ID，如 app.example.files"});
    vec![
        tool(
            LIST,
            "列出家里服务器上已经装的插件：ID、名字、版本、状态和网页地址。",
            json!({"type": "object", "properties": {}}),
            true,
        ),
        tool(
            SEARCH,
            "在插件目录里按关键词找可以安装的插件：ID、名字、说明和要申请的权限。不写关键词时列出全部。",
            json!({"type": "object", "properties": {
                "query": {"type": "string", "description": "关键词，多个用空格分开"},
            }}),
            true,
        ),
        tool(
            INSTALL,
            "安装插件目录里的一个插件。调用前先用一句话告诉主人：确认卡片会发到 Wonton App 里的“Wonton”私聊，\
             请去那里确认。调用会一直等到主人回答或者确认过期（10 分钟）；确认后安装，返回结果。",
            json!({"type": "object", "properties": {"id": id}, "required": ["id"]}),
            false,
        ),
        tool(
            UNINSTALL,
            "卸载一个已经装的插件。调用前先用一句话告诉主人：确认卡片会发到 Wonton App 里的“Wonton”私聊，\
             请去那里确认。调用会一直等到主人回答或者确认过期（10 分钟）。",
            json!({"type": "object", "properties": {
                "id": id,
                "delete_data": {"type": "boolean", "description": "同时删除它的数据，不可恢复；默认保留"},
            }, "required": ["id"]}),
            false,
        ),
    ]
}

fn success(value: &Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}

pub async fn call(bus: &Arc<Bus>, caller: &Caller, tool: &str, args: JsonObject) -> CallToolResult {
    let name = naming::join(naming::CORE, tool);
    let value = Value::Object(args.clone());
    let read = match tool {
        LIST => Ok(list(bus)),
        SEARCH => search(bus, &args),
        INSTALL => return install(bus, caller, &name, args).await,
        UNINSTALL => return uninstall(bus, caller, &name, args).await,
        _ => Err(format!("没有工具 {name}")),
    };
    match read {
        Ok(result) => {
            bus.audit.record(&Record {
                caller: caller.id.as_str(),
                tool: &name,
                arguments: &value,
                decision: Decision::Allowed,
                outcome: Some("ok"),
            });
            success(&result)
        }
        Err(message) => bus.deny(caller, &name, &value, &message),
    }
}

fn list(bus: &Bus) -> Value {
    let plugins: Vec<Value> = bus
        .plugins
        .list()
        .into_iter()
        .map(|p| {
            json!({
                "id": p.id,
                "title": cards::text(&p.title),
                "version": p.version,
                "status": p.status,
                "url": p.url,
            })
        })
        .collect();
    json!({ "plugins": plugins })
}

fn search(bus: &Bus, args: &JsonObject) -> Result<Value, String> {
    let catalog = bus
        .catalog
        .as_ref()
        .ok_or("核心没有配置插件目录（WONTON_CATALOG）")?;
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let found: Vec<Value> = catalog
        .search(query)?
        .iter()
        .map(|package| {
            let m = &package.manifest;
            json!({
                "id": m.id,
                "title": cards::text(&m.title),
                "description": cards::text(&m.description),
                "version": m.version,
                "permissions": m.permissions.iter().map(cards::permission).collect::<Vec<_>>(),
                "installed": bus.plugins.is_installed(&m.id),
                "app_only": m.permissions.iter().any(|p| p.is_high()),
            })
        })
        .collect();
    Ok(json!({ "plugins": found }))
}

fn required<'a>(args: &'a JsonObject, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("缺少参数 {key}"))
}

/// 不需要主人确认就能做的检查：目录里有没有、装没装、权限 Bot 能不能申请、清单合不合法。
fn installable(bus: &Bus, args: &JsonObject) -> Result<PluginPackage, String> {
    let id = required(args, "id")?;
    let catalog = bus
        .catalog
        .as_ref()
        .ok_or("核心没有配置插件目录（WONTON_CATALOG）")?;
    let package = catalog
        .find(id)?
        .ok_or_else(|| format!("插件目录里没有 {id}，先用 wonton__search_catalog 找"))?;
    let manifest = &package.manifest;
    if bus.plugins.is_installed(&manifest.id) {
        return Err(format!("{id} 已经装了"));
    }
    let high: Vec<String> = manifest
        .permissions
        .iter()
        .filter(|p| p.is_high())
        .map(ToString::to_string)
        .collect();
    if !high.is_empty() {
        return Err(format!(
            "{id} 要申请高权限（{}），只能由主人在 Wonton App 里安装",
            high.join("、")
        ));
    }
    dry_run(&package)
        .map_err(InstallError::from)
        .map_err(|e| format!("{id} 现在装不了（{}）：{e}", e.kind()))?;
    Ok(package)
}

async fn install(bus: &Arc<Bus>, caller: &Caller, name: &str, args: JsonObject) -> CallToolResult {
    let value = Value::Object(args.clone());
    let package = match installable(bus, &args) {
        Ok(package) => package,
        Err(message) => return bus.deny(caller, name, &value, &message),
    };
    let draft = cards::install(&caller.name, &package.manifest);
    let permissions = package.manifest.permissions.clone();
    let id = package.manifest.id.clone();
    let run = move |bus: Arc<Bus>| async move {
        match bus.plugins.install_package(package, &permissions).await {
            Ok(view) => {
                let title = cards::text(&view.title);
                let message = match &view.url {
                    Some(url) => format!("已装好 {title}，网页地址 {url}"),
                    None => format!("已装好 {title}"),
                };
                let result = json!({
                    "installed": view.id,
                    "title": title,
                    "version": view.version,
                    "status": view.status,
                    "url": view.url,
                });
                Outcome::ok(result.to_string(), message)
            }
            Err(e) => Outcome::failed(format!("安装 {id} 失败（{}）：{e}", e.kind())),
        }
    };
    bus.confirmed(caller, name, value, draft, run).await
}

async fn uninstall(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    args: JsonObject,
) -> CallToolResult {
    let value = Value::Object(args.clone());
    let checked = required(&args, "id").and_then(|id| {
        let id = PluginId::parse(id).map_err(|e| e.to_string())?;
        let view = bus
            .plugins
            .list()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("没有装 {id}"))?;
        if id == caller.id {
            return Err("不能卸载自己".to_owned());
        }
        if bus.plugins.is_system(&id) {
            return Err(format!("{id} 是系统插件，不能卸载"));
        }
        Ok((id, cards::text(&view.title)))
    });
    let (id, title) = match checked {
        Ok(found) => found,
        Err(message) => return bus.deny(caller, name, &value, &message),
    };
    let purge = args
        .get("delete_data")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let draft = cards::uninstall(&caller.name, &title, purge);
    let run = move |bus: Arc<Bus>| async move {
        match bus.plugins.uninstall(&id, purge).await {
            Ok(report) => {
                let data = if report.data_removed {
                    "，数据已删除"
                } else {
                    "，数据保留"
                };
                let result = json!({"uninstalled": id, "report": report});
                Outcome::ok(result.to_string(), format!("已卸载 {title}{data}"))
            }
            Err(e) => Outcome::failed(format!("卸载 {id} 失败（{}）：{e}", e.kind())),
        }
    };
    bus.confirmed(caller, name, value, draft, run).await
}
