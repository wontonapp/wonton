//! Agent 从 outbox 装和更新自己写的插件（ADR-0033 的“Agent 写插件：从聊天到底栏”）：
//! `wonton__install_plugin` 的 `dir`。`id` 路径也在这里：判定、卡片（最高级多一张）和执行共用，
//! 走的是官方安装/升级的同一层（`install_detached`、`upgrade_upload`），不另写第二套。

use std::os::fd::AsFd;
use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject};
use serde_json::{Value, json};

use super::Outcome;
use super::audit::{Decision, Record};
use super::cards;
use super::{Bus, Caller, error};
use crate::files::share_fs::saving;
use crate::plugin::check;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::{Manifest, Permission};
use crate::plugin::privileges::{self, CapabilityRisk};
use crate::plugin::store::InstalledRecord;
use crate::plugin::{PluginPackage, PluginView, package_files};
use wonton_protocol::PluginWriter;

fn result(value: &Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}

/// 清单里有最高级权限时，安装和更新都要多一张“我知道”的卡片（ADR-0036）。
fn highest(manifest: &Manifest) -> bool {
    manifest
        .permissions
        .iter()
        .any(|p| privileges::risk(p) == Some(CapabilityRisk::Highest))
}

pub(super) async fn install(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    args: JsonObject,
) -> CallToolResult {
    let value = Value::Object(args.clone());
    let id = args.get("id").and_then(Value::as_str);
    let dir = args.get("dir").and_then(Value::as_str);
    match (id, dir) {
        (Some(_), Some(_)) | (None, None) => bus.deny(
            caller,
            name,
            &value,
            "invalid_input：id 和 dir 必须且只能给一个：官方索引里的用 id，自己写的用 dir",
        ),
        (Some(id), None) => official(bus, caller, name, &value, id).await,
        (None, Some(dir)) => from_dir(bus, caller, name, &value, dir).await,
    }
}

/// `id` 路径：官方索引里的插件，和以前一样经确认卡片安装。
async fn official(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    id: &str,
) -> CallToolResult {
    let package = match super::core_tools::installable(bus, id).await {
        Ok(package) => package,
        Err(message) => return bus.deny(caller, name, value, &message),
    };
    let langs = bus.plugins.owner_languages();
    let mut draft = cards::install(&caller.name, &package.manifest, &langs);
    match bus.plugins.retained_data(&package.manifest.id).await {
        Ok(Some(notice)) => draft.lines.insert(1, notice),
        Ok(None) => {}
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &format!("internal_error：查旧数据失败：{e}"),
            );
        }
    }
    let second = highest(&package.manifest).then(|| cards::highest_confirm(&package, &langs));
    let granted = package.manifest.permissions.clone();
    let run = move |bus: Arc<Bus>| async move {
        match bus.plugins.install_detached(package, granted, false).await {
            Ok(view) => installed_outcome(&view, None),
            Err(e) => Outcome::failed(format!("安装失败（{}）：{e}", e.kind())),
        }
    };
    bus.confirmed_twice(caller, name, value.clone(), (draft, second, None), run)
        .await
}

fn installed_outcome(view: &PluginView, note: Option<String>) -> Outcome {
    let title = cards::text(&view.title);
    let mut result = json!({
        "installed": view.id,
        "title": title,
        "version": view.version,
        "status": view.status,
        "url": view.url,
    });
    let message = match &view.url {
        Some(url) => format!("已装好 {title}，网页地址 {url}"),
        None => format!("已装好 {title}"),
    };
    let message = match note {
        Some(note) => {
            result["note"] = json!(note);
            format!("{message}。{note}")
        }
        None => message,
    };
    Outcome::ok(result.to_string(), message)
}

/// `dir` 路径：outbox 里的插件目录，校验、判定（安装还是更新、归谁写），然后卡片或直接更新。
async fn from_dir(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    dir: &str,
) -> CallToolResult {
    let deny = |message: String| bus.deny(caller, name, value, &message);
    let app_data = bus.plugins.app_data_dir(&caller.id);
    // 目录 fd 一路钉住：校验和装的都是这一眼看到的包，中途换成软链接也出不去。
    let opened = match saving::outbox_dir(&app_data, dir) {
        Ok((opened, _)) => opened,
        Err(failure) => return deny(failure.to_string()),
    };
    // 只读一次：对这个包做检查，装的就是校验的这一份，中间改文件换不掉。
    let package = match PluginPackage::load_dir_at(opened.as_fd()) {
        Ok(package) => package,
        Err(e) => return deny(format!("invalid_input：插件包不合法：{e}")),
    };
    // 和 wonton__check_plugin 同一份校验：有 problems 就不发卡片，全部列出来。
    let problems =
        match check::check_package(&package, opened.as_fd(), false, bus.plugins.images()).await {
            Ok(report) => report.problems,
            Err(e) => vec![e],
        };
    if !problems.is_empty() {
        return deny(format!(
            "invalid_input：校验没过，改完再装：{}",
            problems.join("；")
        ));
    }
    // 和 id 路径一样，卡片之前就知道这台主机装不装得了。
    if let Err(e) = bus.plugins.fits_host(&package.manifest).await {
        return deny(format!("invalid_input：{e}"));
    }
    let id = package.manifest.id.clone();
    let langs = bus.plugins.owner_languages();
    let record = match bus.plugins.installed_record(&id) {
        Ok(record) => record,
        Err(e) => return deny(format!("internal_error：读插件记录失败：{e}")),
    };
    match record {
        None => first_install(bus, caller, name, value, package, &langs).await,
        Some(record) => update(bus, caller, name, value, package, record, &langs).await,
    }
}

/// 没装过：安装。卡片写明未审核、全部权限、镜像和以前留下的数据；最高级再多点一张。
async fn first_install(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    package: PluginPackage,
    langs: &Languages,
) -> CallToolResult {
    let id = package.manifest.id.clone();
    // 之前还在等的同一份提交让位：主人不会面对两张都作数的卡。
    bus.confirmations.supersede_pending(&caller.id, &id).await;
    let mut draft = cards::agent_install(&caller.name, &package, langs);
    match bus.plugins.retained_data(&id).await {
        Ok(Some(notice)) => draft.lines.insert(1, notice),
        Ok(None) => {}
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &format!("internal_error：查旧数据失败：{e}"),
            );
        }
    }
    let second = highest(&package.manifest).then(|| cards::highest_confirm(&package, langs));
    let granted = package.manifest.permissions.clone();
    let writer = PluginWriter {
        plugin_id: caller.id.to_string(),
        title: caller.name.clone(),
    };
    let id_for_run = id.clone();
    let run = move |bus: Arc<Bus>| async move {
        match bus.plugins.install_detached(package, granted, false).await {
            Ok(view) => {
                // 归属没记下来时更新就装不上，如实告诉 Agent。
                let note = match bus.plugins.set_writer(&id_for_run, Some(writer)) {
                    Ok(()) => None,
                    Err(e) => {
                        tracing::error!(plugin = %id_for_run, error = %e, "记插件的归属失败");
                        Some(format!("插件归属没记下来（{e}），下次更新会装不上"))
                    }
                };
                installed_outcome(&view, note)
            }
            Err(e) => Outcome::failed(format!("安装 {} 失败（{}）：{}", id_for_run, e.kind(), e)),
        }
    };
    bus.confirmed_twice(caller, name, value.clone(), (draft, second, None), run)
        .await
}

/// 不打扰主人的更新：直接升级，写审计（ADR-0033 的“更新，权限没变宽”）。只有写它的 Bot 能更新；
/// 权限没变宽、没有很高或最高级、主人没开“每次都问”时走这里，发卡片的部分在 [`update`]。
async fn quiet_update(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    package: PluginPackage,
    record: &InstalledRecord,
    granted: Vec<Permission>,
) -> CallToolResult {
    let id = package.manifest.id.clone();
    let version = package.manifest.version.clone();
    let run = bus
        .plugins
        .upgrade_upload(&id, package, granted, record.lan_webui)
        .await;
    bus.audit.record(&Record {
        caller: caller.id.as_str(),
        tool: name,
        arguments: value,
        decision: Decision::Allowed,
        basis: None,
        outcome: if run.is_ok() {
            Some("ok")
        } else {
            Some("failed")
        },
        files: &[],
    });
    match run {
        Ok(()) => result(&json!({
            "updated": id.to_string(),
            "version": version,
            "note": "权限没变宽，没有打扰主人，已写进审计",
        })),
        Err(e) => error(&format!("更新 {} 失败（{}）：{}", id, e.kind(), e)),
    }
}

/// 写审计；其余发卡片，最高级再多点一张。摘要和装着的相同报“没有变化”。
async fn update(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    package: PluginPackage,
    record: InstalledRecord,
    langs: &Languages,
) -> CallToolResult {
    let deny = |message: String| bus.deny(caller, name, value, &message);
    let id = package.manifest.id.clone();
    let version = package.manifest.version.clone();
    let owns = record
        .written_by
        .as_ref()
        .is_some_and(|w| w.plugin_id == caller.id.to_string());
    if !owns {
        return deny(format!(
            "invalid_input：{id} 已经装了，不是你写的插件，不能从这里更新"
        ));
    }
    let installed_digest = package_files::digest(record.source.as_bytes(), &record.files);
    if installed_digest == package.digest() {
        return deny("invalid_input：没有变化：装着的就是这份（摘要相同）".to_owned());
    }
    let risky = highest(&package.manifest)
        || package
            .manifest
            .permissions
            .iter()
            .any(|p| privileges::risk(p) == Some(CapabilityRisk::VeryHigh));
    let widened = cards::added_lines(&record, &package, langs);
    // 授予的就是新清单里写的：减权限时旧授予不作数，没变宽时它是子集。
    let granted = package.manifest.permissions.clone();
    let lan_webui = record.lan_webui;
    if widened.is_empty() && !risky && !record.ask_every_update {
        // 不打扰主人：直接更新，写审计（ADR-0033 的“更新，权限没变宽”）。之前还在等的
        // 同一份提交让位，不能让主人批一份已经被盖掉的。
        bus.confirmations.supersede_pending(&caller.id, &id).await;
        return quiet_update(bus, caller, name, value, package, &record, granted).await;
    }
    // 发卡片前同样让位：同一份插件只留一张等确认的卡。
    bus.confirmations.supersede_pending(&caller.id, &id).await;
    let (header, lines) = if !widened.is_empty() {
        ("这次更新新增的项：", widened)
    } else if risky {
        (
            "这次更新申请了很高或最高级的权限：",
            cards::risky_lines(&package, langs),
        )
    } else {
        ("权限没有变宽，主人打开了“每次都问”：", Vec::new())
    };
    let draft = cards::agent_update(
        &caller.name,
        &package,
        header,
        &lines,
        &record.version,
        langs,
    );
    let second = highest(&package.manifest).then(|| cards::highest_confirm(&package, langs));
    let id_for_run = id.clone();
    let run = move |bus: Arc<Bus>| async move {
        match bus
            .plugins
            .upgrade_upload(&id_for_run, package, granted, lan_webui)
            .await
        {
            Ok(()) => {
                let url = bus
                    .plugins
                    .list()
                    .into_iter()
                    .find(|p| p.id == id_for_run)
                    .and_then(|p| p.url);
                let message = match &url {
                    Some(url) => format!("已更新 {version}，网页地址 {url}"),
                    None => format!("已更新 {version}"),
                };
                Outcome::ok(
                    json!({"updated": id_for_run.to_string(), "version": version, "url": url})
                        .to_string(),
                    message,
                )
            }
            Err(e) => Outcome::failed(format!("更新 {} 失败（{}）：{}", id_for_run, e.kind(), e)),
        }
    };
    bus.confirmed_twice(caller, name, value.clone(), (draft, second, None), run)
        .await
}
