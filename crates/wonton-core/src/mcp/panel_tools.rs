//! `wonton__pin_page`、`wonton__unpin_page`、`wonton__list_pinned_pages`（ADR-0052 的“Bot 往面板上放页面”）。
//!
//! Bot 只能钉到它自己的房间或主面板；页面是已装插件里的一页、公网网页或插件的 MCP Apps 界面。
//! 页面存在服务器核心的钉页记录里，也算进面板 5 页的上限。第一次往主面板钉要在 Bot 的房间里问主人一次，
//! 同意后这个 Bot 以后不再问；钉进自己的房间不问。Bot 钉的页面在 App 里和用户放的合在一起。

use std::collections::HashSet;
use std::sync::Arc;

use rmcp::model::{CallToolResult, JsonObject, Tool};
use serde_json::{Value, json};
use url::Url;
use wonton_protocol::{
    BotPanelPin, BotPanelPins, ConfirmationState, LAYOUT_EVENT, MAX_PANEL_PAGES, PanelScope,
    ROOM_PANEL_EVENT, TabRef,
};

use super::audit::{Decision, Record};
use super::cards;
use super::{Bus, Caller, ToolFailure, core_tools, naming};
use crate::bot_activity::Guard;
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;
use crate::plugin::pins::PanelPin;
use crate::secret::random_token;

pub(super) const PIN: &str = "pin_page";
pub(super) const UNPIN: &str = "unpin_page";
pub(super) const LIST: &str = "list_pinned_pages";

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
        rmcp::model::ToolAnnotations::new()
            .read_only(read_only)
            .destructive(name == UNPIN)
            .open_world(name == PIN),
    )
}

pub(super) fn definitions() -> Vec<Tool> {
    let target = json!({
        "type": "string",
        "enum": ["room", "main"],
        "description": "钉到哪：room 是你自己的房间（默认），main 是主人的主聊天页面板",
    });
    let page = json!({"type": "object", "properties": {
        "plugin": {"type": "string", "description": "插件 ID；配 path 或 resource_uri 用"},
        "path": {"type": "string", "description": "插件网站里的路径，如 /reports/today；只留在这个插件网站里"},
        "url": {"type": "string", "description": "公网网页，只收 https://"},
        "resource_uri": {"type": "string", "description": "MCP Apps 的 ui:// 资源；要同时给提供它的 plugin"},
        "title": {"type": "string", "description": "页面的名字；不写按页面补"},
        "target": target.clone(),
        "attention": {"type": "boolean", "description": "画红点提醒主人看；默认不画"},
    }, "required": []});
    vec![
        tool(
            PIN,
            "把一个页面钉到面板上：你自己房间的面板（默认），或主人的主聊天页面板。页面是已装插件里的一页\
             （plugin 加 path）、公网网页（url），或插件提供的 MCP Apps 界面（plugin 加 resource_uri）。\
             同一个页面再钉一次是更新名字和红点。钉进自己的房间不问；第一次钉进主面板会在你的房间里发确认\
             卡片，调用等到主人回答。面板满了（5 页）会如实返回放不下。",
            page,
            false,
        ),
        tool(
            UNPIN,
            "把你自己钉过的一个页面从面板上拿掉。参数和钉的时候一样（页面加 target）。",
            json!({"type": "object", "properties": {
                "plugin": {"type": "string"},
                "path": {"type": "string"},
                "url": {"type": "string"},
                "resource_uri": {"type": "string"},
                "target": target,
            }, "required": []}),
            false,
        ),
        tool(
            LIST,
            "列出你自己钉在面板上的页面和钉到哪，以及主人有没有同意你往主面板钉页。",
            json!({"type": "object", "properties": {}}),
            true,
        ),
    ]
}

pub(super) async fn call(
    bus: &Arc<Bus>,
    caller: &Caller,
    tool: &str,
    args: JsonObject,
    activity: Guard,
) -> CallToolResult {
    let name = naming::join(naming::CORE, tool);
    let value = Value::Object(args.clone());
    let outcome = match tool {
        PIN => pin(bus, caller, &args, activity).await,
        UNPIN => unpin(bus, caller, &args),
        LIST => list(bus, caller),
        _ => Err(ToolFailure::new(
            "invalid_input",
            format!("没有工具 {name}"),
        )),
    };
    match outcome {
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
            core_tools::success(&result)
        }
        Err(failure) => bus.deny(caller, &name, &value, &failure),
    }
}

#[derive(Clone)]
struct BotContext {
    plugin: PluginId,
    bot: String,
    name: String,
    room: String,
}

fn context(bus: &Bus, caller: &Caller) -> Result<BotContext, ToolFailure> {
    let Some(identity) = &caller.bot else {
        return Err(ToolFailure::new(
            "invalid_input",
            l10n::text(
                &bus.owner_languages(),
                "panel-pin-bot-only",
                &[("plugin", caller.id.as_str().into())],
            ),
        ));
    };
    let entry = bus
        .plugins
        .store()
        .bots()
        .entry(&caller.id, &identity.bot)
        .map_err(storage)?
        .ok_or_else(|| {
            ToolFailure::new(
                "not_found",
                l10n::text(
                    &bus.owner_languages(),
                    "panel-pin-bot-missing",
                    &[("bot", identity.bot.as_str().into())],
                ),
            )
        })?;
    let name = bus
        .plugins
        .bot_name(&caller.id, &identity.bot)
        .map(|name| cards::text(&name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| caller.name.clone());
    Ok(BotContext {
        plugin: caller.id.clone(),
        bot: identity.bot.clone(),
        name,
        room: entry.account.home_room,
    })
}

fn storage(error: impl std::fmt::Display) -> ToolFailure {
    ToolFailure::new("storage", error.to_string())
}

fn target(args: &JsonObject) -> Result<String, ToolFailure> {
    match args.get("target").and_then(Value::as_str) {
        None | Some("room") => Ok("room".to_owned()),
        Some("main") => Ok("main".to_owned()),
        Some(other) => Err(ToolFailure::new(
            "invalid_input",
            format!("target 只能是 room 或 main，拿到 {other}"),
        )),
    }
}

fn scope_of(context: &BotContext, target: &str) -> PanelScope {
    if target == "main" {
        PanelScope::Main
    } else {
        PanelScope::Room {
            room_id: context.room.clone(),
        }
    }
}

/// 参数里的页面：必须正好给一种，插件页和资源要装在、跑着。
fn page(bus: &Bus, args: &JsonObject) -> Result<(TabRef, String), ToolFailure> {
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    let kinds = ["url", "resource_uri"]
        .iter()
        .filter(|key| text(key).is_some_and(|value| !value.trim().is_empty()))
        .count()
        + usize::from(text("path").is_some());
    if kinds != 1 {
        return Err(ToolFailure::new(
            "invalid_input",
            "页面要给 url、resource_uri，或 plugin 加 path，只给一种",
        ));
    }
    if let Some(url) = text("url") {
        return url_page(url);
    }
    let Some(id) = text("plugin") else {
        return Err(ToolFailure::new("invalid_input", "要给提供页面的 plugin"));
    };
    let plugin = PluginId::parse(id)
        .map_err(|_| ToolFailure::new("invalid_input", format!("插件 ID 不对：{id}")))?;
    let title = bus
        .plugins
        .title(&plugin)
        .map(|title| cards::text(&title))
        .unwrap_or_else(|| plugin.to_string());
    if let Some(uri) = text("resource_uri") {
        let uri = uri.trim();
        if !uri.starts_with("ui://") {
            return Err(ToolFailure::new(
                "invalid_input",
                format!("resource_uri 要以 ui:// 开头，拿到 {uri}"),
            ));
        }
        if !bus.plugins.mcp_servers().iter().any(|s| s.id == plugin) {
            return Err(ToolFailure::new(
                "source_unavailable",
                format!("插件 {plugin} 没有在运行的 MCP 服务，界面调不出来"),
            ));
        }
        return Ok((
            TabRef::UiResource {
                plugin_id: plugin.as_str().to_owned(),
                resource_uri: uri.to_owned(),
            },
            title,
        ));
    }
    let path = text("path").unwrap_or_default().trim();
    if !bus.plugins.has_webui(&plugin) {
        return Err(ToolFailure::new(
            "not_found",
            format!("插件 {plugin} 没有网页，钉不了插件里的页"),
        ));
    }
    let path = if path.is_empty() || path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    Ok((
        TabRef::PluginPath {
            plugin_id: plugin.as_str().to_owned(),
            path,
        },
        title,
    ))
}

/// 取消钉页时只按参数算出页面，不要求插件还装着、跑着：停了也要能拿掉。
fn locate(args: &JsonObject) -> Result<TabRef, ToolFailure> {
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    if let Some(url) = text("url") {
        return Ok(url_page(url)?.0);
    }
    let Some(plugin_id) = text("plugin") else {
        return Err(ToolFailure::new("invalid_input", "要给页面"));
    };
    if let Some(uri) = text("resource_uri") {
        return Ok(TabRef::UiResource {
            plugin_id: plugin_id.to_owned(),
            resource_uri: uri.trim().to_owned(),
        });
    }
    let path = text("path").unwrap_or_default().trim();
    let path = if path.is_empty() || path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    Ok(TabRef::PluginPath {
        plugin_id: plugin_id.to_owned(),
        path,
    })
}

fn url_page(url: &str) -> Result<(TabRef, String), ToolFailure> {
    let parsed = Url::parse(url.trim())
        .map_err(|_| ToolFailure::new("invalid_input", format!("网址不对：{url}")))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(ToolFailure::new(
            "invalid_input",
            format!("只收 https:// 网址，拿到 {url}"),
        ));
    }
    let title = parsed.host_str().unwrap_or(url).to_owned();
    Ok((
        TabRef::Url {
            url: parsed.to_string(),
        },
        title,
    ))
}

/// 面板里已经有几种页面：用户账号数据里的加上所有 Bot 钉的。按页面去重（同一个页面不重复占名额）。
async fn taken(bus: &Bus, scope: &PanelScope) -> Result<HashSet<TabRef>, ToolFailure> {
    let mut pages = HashSet::new();
    for pins in bus
        .plugins
        .store()
        .pins()
        .all()
        .map_err(storage)?
        .into_values()
    {
        for entry in pins.bots.into_values() {
            pages.extend(
                entry
                    .pins
                    .into_iter()
                    .filter(|pin| pin.scope == *scope)
                    .map(|pin| pin.tab),
            );
        }
    }
    pages.extend(user_pages(bus, scope).await?);
    Ok(pages)
}

/// 用户自己放的页面：主面板在全局账号数据，房间面板在房间账号数据。读不到时如实报错，不当作空。
async fn user_pages(bus: &Bus, scope: &PanelScope) -> Result<Vec<TabRef>, ToolFailure> {
    let cards = bus
        .cards()
        .map_err(|e| ToolFailure::new("service_unavailable", e))?;
    let langs = bus.owner_languages();
    let Some(matrix) = cards.matrix.clone() else {
        return Err(ToolFailure::new(
            "service_unavailable",
            l10n::text(
                &langs,
                "panel-pin-matrix",
                &[("error", "核心没有接上 Matrix".into())],
            ),
        ));
    };
    let login = matrix
        .login(&cards.owner, "Wonton 核心（读面板）")
        .await
        .map_err(|e| {
            ToolFailure::new(
                "service_unavailable",
                l10n::text(
                    &langs,
                    "panel-pin-matrix",
                    &[("error", e.to_string().into())],
                ),
            )
        })?;
    let session = matrix.session(&login);
    let read = match scope {
        PanelScope::Main => session.account_data(LAYOUT_EVENT).await,
        PanelScope::Room { room_id } => session.room_account_data(room_id, ROOM_PANEL_EVENT).await,
    };
    let _ = session.logout().await;
    let value = read
        .map_err(|e| ToolFailure::new("service_unavailable", e.to_string()))?
        .unwrap_or(Value::Null);
    let tabs = match scope {
        PanelScope::Main => value["panel"].as_array().map(|entries| {
            entries
                .iter()
                .filter_map(|entry| serde_json::from_value(entry["tab"].clone()).ok())
                .collect()
        }),
        PanelScope::Room { .. } => value["pages"].as_array().map(|entries| {
            entries
                .iter()
                .filter_map(|entry| serde_json::from_value(entry["tab"].clone()).ok())
                .collect()
        }),
    };
    Ok(tabs.unwrap_or_default())
}

fn words(bus: &Bus, key: &str, args: &[(&str, wonton_l10n::Arg)]) -> String {
    l10n::text(&bus.owner_languages(), key, args)
}

fn existing<'a>(
    entry: &'a mut crate::plugin::pins::BotPins,
    scope: &PanelScope,
    tab: &TabRef,
) -> Option<&'a mut PanelPin> {
    entry
        .pins
        .iter_mut()
        .find(|pin| pin.scope == *scope && pin.tab == *tab)
}

fn count_of(pages: &HashSet<TabRef>) -> usize {
    pages.len()
}

async fn pin(
    bus: &Arc<Bus>,
    caller: &Caller,
    args: &JsonObject,
    activity: Guard,
) -> Result<Value, ToolFailure> {
    let context = context(bus, caller)?;
    let target = target(args)?;
    let scope = scope_of(&context, &target);
    let (tab, fallback) = page(bus, args)?;
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .unwrap_or(fallback);
    let attention = args
        .get("attention")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    drop(activity);
    let store = bus.plugins.store().pins();
    let record = store.load(&context.plugin).map_err(storage)?;
    let bot = context.bot.clone();
    let updated = store
        .with(&context.plugin, |record| {
            let entry = record.bots.entry(bot.clone()).or_default();
            if let Some(pin) = existing(entry, &scope, &tab) {
                pin.title = title.clone();
                pin.attention = attention;
                return Some(pin.clone());
            }
            None
        })
        .map_err(storage)?;
    if let Some(pin) = updated {
        return Ok(json!({
            "id": pin.id,
            "updated": true,
            "title": pin.title,
            "attention": pin.attention,
        }));
    }
    let mut pages = taken(bus, &scope).await?;
    if count_of(&pages) >= MAX_PANEL_PAGES {
        return Err(ToolFailure::new(
            "panel_full",
            words(bus, "panel-pin-full", &[("max", MAX_PANEL_PAGES.into())]),
        ));
    }
    pages.insert(tab.clone());
    let consented = record
        .bots
        .get(&context.bot)
        .is_some_and(|entry| entry.main_consent);
    let needs_consent = target == "main" && !consented;
    let pin = PanelPin {
        id: random_token(),
        scope: scope.clone(),
        tab,
        title,
        attention,
    };
    let answer = json!({
        "id": pin.id,
        "target": target,
        "scope": scope,
        "title": pin.title,
        "attention": pin.attention,
        "count": pages.len(),
    });
    if !needs_consent {
        save_pin(bus, &context, pin, false).await?;
        return Ok(answer);
    }
    // 同意要等主人：放进独立任务，Agent 断开、这次调用被取消时卡片照样等，答应后页面照样钉上。
    let bus = bus.clone();
    let context = context.clone();
    let task = tokio::spawn(async move {
        let _guard = PinCardGuard::new(&bus, &context.plugin)?;
        confirm_main(&bus, &context).await?;
        save_pin(&bus, &context, pin, true).await?;
        Ok(answer)
    });
    task.await
        .map_err(|e| ToolFailure::new("internal", format!("钉页的确认意外中止：{e}")))?
}

/// 把钉页写进记录：同一页面已经在了就更新，否则追加；`consented` 时同时记下同意。
async fn save_pin(
    bus: &Bus,
    context: &BotContext,
    pin: PanelPin,
    consented: bool,
) -> Result<(), ToolFailure> {
    let bot = context.bot.clone();
    bus.plugins
        .store()
        .pins()
        .with(&context.plugin, |record| {
            let entry = record.bots.entry(bot).or_default();
            if consented {
                entry.main_consent = true;
            }
            if let Some(found) = existing(entry, &pin.scope, &pin.tab) {
                found.title = pin.title.clone();
                found.attention = pin.attention;
            } else {
                entry.pins.push(pin);
            }
        })
        .map_err(storage)
}

/// 正在问主人同意时占着这个插件：任务被取消也会在退出时清掉，不留下永远“已经在问”。
struct PinCardGuard {
    bus: Arc<Bus>,
    plugin: PluginId,
}

impl PinCardGuard {
    fn new(bus: &Arc<Bus>, plugin: &PluginId) -> Result<Self, ToolFailure> {
        let mut pending = bus.pin_cards.lock().expect("钉页卡片表锁中毒");
        if !pending.insert(plugin.clone()) {
            return Err(ToolFailure::new(
                "too_many_pending",
                words(bus, "panel-pin-already-asking", &[]),
            ));
        }
        drop(pending);
        Ok(Self {
            bus: bus.clone(),
            plugin: plugin.clone(),
        })
    }
}

impl Drop for PinCardGuard {
    fn drop(&mut self) {
        self.bus
            .pin_cards
            .lock()
            .expect("钉页卡片表锁中毒")
            .remove(&self.plugin);
    }
}

/// 第一次往主面板钉：请主人确认，等到主人回答。同一个插件同时只问一张。
/// 卡片显示在这个会话里（ADR-0016）：发起者是这个 Bot，来源是它收到调用的房间。
async fn confirm_main(bus: &Arc<Bus>, context: &BotContext) -> Result<(), ToolFailure> {
    let langs = bus.owner_languages();
    let requester = wonton_protocol::ConfirmationRequester::Bot {
        bot: wonton_protocol::BotRef {
            plugin_id: context.plugin.to_string(),
            bot: context.bot.clone(),
        },
        user_id: bot_user(bus, context)?,
    };
    let origin = Some(wonton_protocol::ConfirmationOrigin {
        room_id: context.room.clone(),
        thread_id: None,
    });
    // 主面板是主人的：卡片发给主人。
    let audience = super::Audience {
        recipients: super::Recipients::Keeper(None),
        requester: Some(requester),
        origin,
        category: None,
    };
    let asked = bus
        .confirmations
        .ask_as(&context.plugin, cards_for(&context.name, &langs), audience)
        .await;
    match asked {
        Ok(ticket) => {
            bus.confirmations
                .finish(
                    ticket,
                    ConfirmationState::Succeeded {
                        message: l10n::text(&bus.owner_languages(), "panel-pin-allowed", &[]),
                    },
                )
                .await;
            Ok(())
        }
        Err(refusal) => Err(ToolFailure::new(refusal.kind(), refusal.message())),
    }
}

fn cards_for(name: &str, langs: &Languages) -> super::Draft {
    let title = l10n::text(langs, "panel-pin-card-title", &[("bot", name.into())]);
    let note = l10n::text(langs, "panel-pin-card-line", &[]);
    let allow = l10n::text(langs, "chat-choice-allow", &[]);
    let deny = l10n::text(langs, "chat-choice-deny", &[]);
    cards::pin_main(title, note, &allow, &deny)
}

fn bot_user(bus: &Bus, context: &BotContext) -> Result<String, ToolFailure> {
    let entry = bus
        .plugins
        .store()
        .bots()
        .entry(&context.plugin, &context.bot)
        .map_err(storage)?
        .ok_or_else(|| ToolFailure::new("not_found", "Bot 记录不见了"))?;
    Ok(entry.account.user_id)
}

fn unpin(bus: &Arc<Bus>, caller: &Caller, args: &JsonObject) -> Result<Value, ToolFailure> {
    let context = context(bus, caller)?;
    let target = target(args)?;
    let scope = scope_of(&context, &target);
    let tab = locate(args)?;
    let store = bus.plugins.store().pins();
    let bot = context.bot.clone();
    let removed = store
        .with(&context.plugin, |record| {
            let Some(entry) = record.bots.get_mut(&bot) else {
                return false;
            };
            let before = entry.pins.len();
            entry
                .pins
                .retain(|pin| pin.scope != scope || pin.tab != tab);
            entry.pins.len() != before
        })
        .map_err(storage)?;
    if !removed {
        return Err(not_pinned(bus));
    }
    Ok(json!({"removed": true}))
}

fn not_pinned(bus: &Bus) -> ToolFailure {
    ToolFailure::new("not_found", words(bus, "panel-pin-not-there", &[]))
}

fn list(bus: &Arc<Bus>, caller: &Caller) -> Result<Value, ToolFailure> {
    let context = context(bus, caller)?;
    let record = bus
        .plugins
        .store()
        .pins()
        .load(&context.plugin)
        .map_err(storage)?;
    let entry = record.bots.get(&context.bot).cloned().unwrap_or_default();
    let pins: Vec<BotPanelPin> = entry
        .pins
        .into_iter()
        .map(|pin| BotPanelPin {
            id: pin.id,
            scope: pin.scope,
            tab: pin.tab,
            title: pin.title,
            attention: pin.attention,
            plugin_id: context.plugin.as_str().to_owned(),
            bot: context.bot.clone(),
            name: context.name.clone(),
        })
        .collect();
    serde_json::to_value(BotPanelPins {
        pins,
        consents: vec![wonton_protocol::BotPinOwner {
            plugin_id: context.plugin.as_str().to_owned(),
            bot: context.bot.clone(),
            name: context.name.clone(),
            main_consent: record.bots[&context.bot].main_consent,
        }],
    })
    .map_err(|e| ToolFailure::new("storage", e.to_string()))
}
