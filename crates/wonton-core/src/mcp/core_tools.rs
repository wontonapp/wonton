//! 核心自己的工具（插件规范的“工具与 MCP 总线”）：列出插件、在官方索引里找插件、安装和卸载、帮助文档，文件工具在
//! [`file_tools`]，存文件在 [`save_tools`]，Bot 自己的资料工具在 [`profile_tools`]（ADR-0044），Agent 写插件用的只读工具
//! （说明、校验、状态，ADR-0033）在 [`authoring_tools`]。安装和卸载要主人确认，高权限也一样，主人点了就装（ADR-0033）。Bot 经 `id` 只能装官方
//! 索引里的插件。

use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool, ToolAnnotations};
use serde_json::{Value, json};
use wonton_protocol::SecretValues;

use super::audit::{Decision, Record};
use super::{
    Bus, Caller, Outcome, ToolFailure, agent_install, authoring_tools, cards, file_tools, naming,
    people_tools, profile_tools, save_tools,
};
use crate::bot_activity::Guard;
use crate::l10n;
use crate::plugin::manifest::{Manifest, Permission, PluginKind};
use crate::plugin::render::dry_run;
use crate::plugin::storefront::index::Listed;
use crate::plugin::{InstallError, PluginId, PluginPackage};
use wonton_protocol::ConfirmationState;

const LIST: &str = "list_plugins";
const SEARCH: &str = "search_catalog";
const INSTALL: &str = "install_plugin";
const UNINSTALL: &str = "uninstall_plugin";
const SET_PERMISSION: &str = "set_plugin_permission";
const HELP: &str = "help";

fn schema(value: Value) -> Arc<JsonObject> {
    match value {
        Value::Object(map) => Arc::new(map),
        _ => unreachable!("工具的参数表是 JSON 对象"),
    }
}

pub(super) fn tool(name: &str, description: &str, input: Value, read_only: bool) -> Tool {
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
    let dir_param =
        json!({"type": "string", "description": "outbox 里的相对路径，如 myapp；装的是你写的插件"});
    let mut tools = vec![
        tool(
            LIST,
            "列出家里服务器上已经装的插件：ID、名字、版本、状态和网页地址。",
            json!({"type": "object", "properties": {}}),
            true,
        ),
        tool(
            SEARCH,
            "在官方索引里按关键词找可以安装的插件：ID、名字、说明和要申请的权限。不写关键词时列出全部。",
            json!({"type": "object", "properties": {
                "query": {"type": "string", "description": "关键词，多个用空格分开"},
            }}),
            true,
        ),
        tool(
            INSTALL,
            "安装一个插件。`id` 是官方索引里的插件；`dir` 是你自己写的、放在 outbox 里的插件目录\
             （先用 wonton__plugin_guide 读说明、wonton__check_plugin 校验到没有 problems）。两种安装都要主人确认：\
             调用前告诉主人的是“我会发起安装，确认卡片会发到 Wonton App 里的“Wonton”私聊”，主人点了允许才会装。\
             调用会一直等到主人回答或者确认过期（10 分钟）。装没装上以这条调用返回的结果为准：说失败就是没装上，\
             把原因和下一步照实转告；等结果的期间主人问起来，说正在等确认。",
            json!({"type": "object", "properties": {"id": id, "dir": dir_param}, "required": []}),
            false,
        ),
        tool(
            UNINSTALL,
            "卸载一个已经装的插件。调用前先告诉主人你会发起卸载，确认卡片会发到 Wonton App 里的“Wonton”私聊。\
             调用会一直等到主人回答或者确认过期（10 分钟）；卸没卸掉以返回的结果为准，照实转告。",
            json!({"type": "object", "properties": {
                "id": id,
                "delete_data": {"type": "boolean", "description": "同时删除它的数据，不可恢复；默认保留"},
            }, "required": ["id"]}),
            false,
        ),
    ];
    tools.push(tool(
        SET_PERMISSION,
        "打开或关掉一个插件的可选权限（如 docker.socket）。打开每次都要主人点确认卡片，关掉是收权限、可以按你的\
         放宽直接做。调用前先告诉主人你会发起这项改动，卡片会发到“Wonton”私聊；改没改成以返回的结果为准，照实转告。",
        json!({"type": "object", "properties": {
            "id": id,
            "permission": {"type": "string", "description": "权限写法，如 docker.socket"},
            "enabled": {"type": "boolean", "description": "true 打开，false 关掉"},
        }, "required": ["id", "permission", "enabled"]}),
        false,
    ));
    tools.push(tool(
        HELP,
        "查 Wonton 和 Wonton App 的帮助文档：怎么加设备、接模型、装应用、用底栏和确认卡片、升级、备份、连不上怎么办。\
         主人问到这些时先查这里再回答，不要凭印象说。不写 topic 时返回目录，写了返回那一页。",
        json!({"type": "object", "properties": {
            "topic": {"type": "string", "description": "目录里的主题，如 devices"},
            "lang": {"type": "string", "description": "语言，如 zh、en；不写时用 App 的语言"},
        }}),
        true,
    ));
    tools.extend(file_tools::definitions());
    tools.extend(save_tools::definitions());
    tools.extend(authoring_tools::definitions());
    tools.extend(profile_tools::definitions());
    tools.extend(people_tools::definitions());
    tools
}

fn success(value: &Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}

pub async fn call(
    bus: &Arc<Bus>,
    caller: &Caller,
    tool: &str,
    args: JsonObject,
    activity: Guard,
) -> CallToolResult {
    let name = naming::join(naming::CORE, tool);
    let value = Value::Object(args.clone());
    let read = match tool {
        LIST => Ok(list(bus)),
        SEARCH => search(bus, &args).await,
        INSTALL => return agent_install::install(bus, caller, &name, args, activity).await,
        UNINSTALL => return uninstall(bus, caller, &name, args, activity).await,
        SET_PERMISSION => return set_permission(bus, caller, &name, args, activity).await,
        HELP => help(bus, &args).await,
        authoring_tools::GUIDE => Ok(authoring_tools::guide()),
        authoring_tools::CHECK => authoring_tools::check(bus, caller, &args).await,
        authoring_tools::STATUS => authoring_tools::status(bus, caller, &args).await,
        file_tools::VIEW | file_tools::SHARE => {
            return file_tools::call(bus, caller, tool, args).await;
        }
        save_tools::LIST => save_tools::list(bus).await,
        save_tools::SAVE => return save_tools::save(bus, caller, args, activity).await,
        profile_tools::GET | profile_tools::SET => {
            return profile_tools::call(bus, caller, tool, args).await;
        }
        people_tools::USER_LOCALE => people_tools::call(bus, caller, &args),
        _ => Err(ToolFailure::new(
            "invalid_input",
            format!("没有工具 {name}"),
        )),
    };
    match read {
        Ok(result) => {
            bus.audit.record(&Record {
                caller: caller.id.as_str(),
                tool: &name,
                arguments: &value,
                decision: Decision::Allowed,
                basis: None,
                outcome: Some("ok"),
                files: &[],
            });
            success(&result)
        }
        Err(failure) => bus.deny(caller, &name, &value, &failure),
    }
}

async fn help(bus: &Bus, args: &JsonObject) -> Result<Value, ToolFailure> {
    let Some(help) = bus.help.get() else {
        return Err(ToolFailure::new(
            "service_unavailable",
            "帮助文档没有接上，过一会再试",
        ));
    };
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    let answer = help.answer(text("topic"), text("lang")).await;
    answer.map_err(|e| ToolFailure::new(e.kind(), e.to_string()))
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
                "tools_error": bus.upstreams.tools_problem(&p.id),
            })
        })
        .collect();
    json!({ "plugins": plugins })
}

/// 官方索引里 Bot 能看到的插件（系统插件随核心预装，哪种来源都装不了），和读索引时遇到的问题。一个都读不到又
/// 有问题时报错，免得 Bot 以为索引是空的。
async fn official(bus: &Bus) -> Result<(Vec<Listed>, Option<String>), ToolFailure> {
    let (index, problem) = bus.plugins.official_index().await;
    let problem = problem.map(|p| p.details);
    let listed: Vec<Listed> = index
        .plugins
        .into_iter()
        .filter(|l| l.manifest.kind != PluginKind::System)
        .collect();
    match problem {
        Some(problem) if listed.is_empty() => Err(ToolFailure::new(
            "source_unavailable",
            format!("官方索引用不了：{problem}"),
        )),
        problem => Ok((listed, problem)),
    }
}

async fn search(bus: &Bus, args: &JsonObject) -> Result<Value, ToolFailure> {
    let words: Vec<String> = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let (listed, problem) = official(bus).await?;
    // 权限的说明按主人的语言写：这个工具是 Agent 替主人调的，请求里没有 Accept-Language。
    let langs = bus.plugins.owner_languages();
    let found: Vec<Value> = listed
        .iter()
        .map(|l| &l.manifest)
        .filter(|m| {
            let text = searchable(m);
            words.iter().all(|word| text.contains(word))
        })
        .map(|m| {
            json!({
                "id": m.id,
                "title": cards::text(&m.title),
                "description": cards::text(&m.description),
                "version": m.version,
                "permissions": m
                    .permissions
                    .iter()
                    .map(|p| cards::permission(p, &langs))
                    .collect::<Vec<_>>(),
                "installed": bus.plugins.is_installed(&m.id),
            })
        })
        .collect();
    let mut result = json!({ "plugins": found });
    if let Some(problem) = problem {
        result["problem"] = json!(problem);
    }
    Ok(result)
}

/// 每个词都出现在 ID、名字或说明里才算找到（不分大小写）。
fn searchable(manifest: &Manifest) -> String {
    let texts = [&manifest.title, &manifest.description];
    let mut text = manifest.id.to_string();
    for value in texts.iter().flat_map(|t| t.values()) {
        text.push(' ');
        text.push_str(value);
    }
    text.to_lowercase()
}

fn required<'a>(args: &'a JsonObject, key: &str) -> Result<&'a str, ToolFailure> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ToolFailure::new("invalid_input", format!("缺少参数 {key}")))
}

/// 不需要主人确认就能做的检查：官方索引里有没有、装没装、这台主机放不放得下、插件包取不取得到、合不合法。
/// 文字都从“没有装上”说起：模型先读到结论，再读哪一步、为什么、能做什么。
pub(super) async fn installable(bus: &Bus, id: &str) -> Result<PluginPackage, ToolFailure> {
    let (listed, _) = official(bus).await?;
    let listed = listed
        .iter()
        .find(|l| l.manifest.id.as_str() == id)
        .ok_or_else(|| {
            ToolFailure::new(
                "not_found",
                format!(
                    "没有装上 {id}：官方索引里没有它。用 wonton__search_catalog 找到正确的 ID 再装"
                ),
            )
        })?;
    let manifest = &listed.manifest;
    if bus.plugins.is_installed(&manifest.id) {
        return Err(ToolFailure::new(
            "conflict",
            format!("{id} 已经装了，不用再装"),
        ));
    }
    let langs = bus.plugins.owner_languages();
    bus.plugins
        .fits_host(manifest, &langs)
        .await
        .map_err(|e| ToolFailure::new(e.kind(), format!("没有装上 {id}（{}）：{e}", e.kind())))?;
    let package = bus.plugins.official_package(listed).await.map_err(|e| {
        let mut message = format!("没有装上 {id}（{}）：{e}", e.kind());
        if e.kind() == "source_unavailable" {
            message.push_str("。可能是连插件源慢，稍后再让我试一次");
        }
        ToolFailure::new(e.kind(), message)
    })?;
    dry_run(&package, &langs)
        .map_err(InstallError::from)
        .map_err(|e| ToolFailure::new(e.kind(), format!("没有装上 {id}（{}）：{e}", e.kind())))?;
    if bus.plugins.missing_profile(&package, &langs).is_some() {
        return Err(ToolFailure::new(
            "owner_profile_incomplete",
            format!("没有装上 {id}：它要用主人的邮箱，主人在 App 的设置里填好邮箱后我再装"),
        ));
    }
    Ok(package)
}

/// 开关一项可选权限的参数和现状检查：装没装、有没有把这项权限声明成可选。
async fn checked_permission(
    bus: &Bus,
    args: &JsonObject,
) -> Result<
    (
        PluginId,
        Permission,
        bool,
        wonton_protocol::PermissionLine,
        String,
    ),
    ToolFailure,
> {
    let id = required(args, "id")?;
    let id = PluginId::parse(id).map_err(|e| ToolFailure::new("invalid_input", e.to_string()))?;
    let enabled = args
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| ToolFailure::new("invalid_input", "enabled 要是 true 或 false"))?;
    let permission = required(args, "permission")?
        .parse::<Permission>()
        .map_err(|e| ToolFailure::new("invalid_input", e.to_string()))?;
    let view = bus
        .plugins
        .list()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ToolFailure::new("not_installed", format!("没有装 {id}")))?;
    let line = bus
        .plugins
        .installed_detail(&id, &bus.plugins.owner_languages())
        .ok_or_else(|| ToolFailure::new("internal_error", format!("{id} 的详情读不到")))?
        .permissions
        .into_iter()
        .find(|line| line.permission == permission.to_string() && line.optional)
        .ok_or_else(|| {
            ToolFailure::new(
                "invalid_input",
                format!("{id} 没有把 {permission} 声明成可选"),
            )
        })?;
    Ok((id, permission, enabled, line, cards::text(&view.title)))
}

/// 开关一项可选权限（ADR-0057）：打开必发卡片、不能被放宽；关掉按 ADR-0024 的放宽走。
async fn set_permission(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    args: JsonObject,
    activity: Guard,
) -> CallToolResult {
    let value = Value::Object(args.clone());
    let checked = checked_permission(bus, &args).await;
    let (id, permission, enabled, line, title) = match checked {
        Ok(found) => found,
        Err(failure) => return bus.deny(caller, name, &value, &failure),
    };
    let langs = bus.plugins.owner_languages();
    let highest = line.risk == Some(wonton_protocol::PermissionRisk::Highest);
    let bot = crate::plugin::store::OpenedBy::Bot {
        plugin_id: caller.id.to_string(),
        title: caller.name.clone(),
    };
    let run = {
        let (id, title, langs) = (id.clone(), title.clone(), langs.clone());
        move |bus: Arc<Bus>, _secrets: SecretValues| async move {
            match bus
                .plugins
                .set_plugin_permission(
                    id.clone(),
                    permission.clone(),
                    enabled,
                    bot,
                    langs.clone(),
                )
                .await
            {
                Ok(_) => Outcome::ok(
                    json!({"id": id.to_string(), "permission": permission.to_string(), "enabled": enabled})
                        .to_string(),
                    l10n::text(&langs, "chat-permission-done", &[("title", title.into())]),
                ),
                Err(e) => Outcome::failed(
                    e.kind(),
                    l10n::text(
                        &langs,
                        "chat-permission-failed",
                        &[("id", id.to_string().into()), ("error", e.to_string().into())],
                    ),
                ),
            }
        }
    };
    // 关掉是收权限：已放宽的调用不再发卡。
    if enabled {
        let draft = cards::set_permission(&caller.name, &title, &line, enabled, highest, &langs);
        return bus
            .confirmed(caller, name, value, (draft, activity), run)
            .await;
    }
    let cleared = bus
        .plugins
        .tool_policy()
        .clearance(caller.id.as_str(), naming::CORE, SET_PERMISSION)
        .is_some();
    if cleared {
        let outcome = run(Arc::clone(bus), SecretValues::new()).await;
        let failed = matches!(outcome.state, ConfirmationState::Failed { .. });
        bus.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: &value,
            decision: Decision::Allowed,
            basis: None,
            outcome: Some(if failed { "failed" } else { "ok" }),
            files: &[],
        });
        return outcome.result;
    }
    let draft = cards::set_permission(&caller.name, &title, &line, enabled, highest, &langs);
    bus.confirmed(caller, name, value, (draft, activity), run)
        .await
}

async fn uninstall(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    args: JsonObject,
    activity: Guard,
) -> CallToolResult {
    let value = Value::Object(args.clone());
    let checked = required(&args, "id").and_then(|id| {
        let id =
            PluginId::parse(id).map_err(|e| ToolFailure::new("invalid_input", e.to_string()))?;
        let view = bus
            .plugins
            .list()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| ToolFailure::new("not_installed", format!("没有装 {id}")))?;
        if id == caller.id {
            return Err(ToolFailure::new("invalid_input", "不能卸载自己"));
        }
        if bus.plugins.is_system(&id) {
            return Err(ToolFailure::new(
                "unsupported",
                format!("{id} 是系统插件，不能卸载"),
            ));
        }
        Ok((id, cards::text(&view.title)))
    });
    let (id, title) = match checked {
        Ok(found) => found,
        Err(failure) => return bus.deny(caller, name, &value, &failure),
    };
    let purge = args
        .get("delete_data")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let langs = bus.plugins.owner_languages();
    let draft = cards::uninstall(&caller.name, &title, purge, &langs);
    let run = move |bus: Arc<Bus>, _secrets: SecretValues| async move {
        match bus
            .plugins
            .uninstall_detached(id.clone(), purge, bus.plugins.owner_languages())
            .await
        {
            Ok(report) => {
                let message = if report.data_removed {
                    l10n::text(
                        &langs,
                        "chat-uninstall-done-purge",
                        &[("title", title.as_str().into())],
                    )
                } else {
                    l10n::text(
                        &langs,
                        "chat-uninstall-done-keep",
                        &[("title", title.as_str().into())],
                    )
                };
                let result = json!({"uninstalled": id, "report": report});
                Outcome::ok(result.to_string(), message)
            }
            Err(e) => Outcome::failed(
                e.kind(),
                l10n::text(
                    &langs,
                    "chat-uninstall-failed",
                    &[
                        ("id", id.as_str().into()),
                        ("kind", e.kind().into()),
                        ("error", e.to_string().into()),
                    ],
                ),
            ),
        }
    };
    bus.confirmed(caller, name, value, (draft, activity), run)
        .await
}
