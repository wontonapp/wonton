//! Agent 从 outbox 装和更新自己写的插件（ADR-0033 的“Agent 写插件：从聊天到底栏”）：
//! `wonton__install_plugin` 的 `dir`。`id` 路径也在这里：判定、卡片（最高级多一张）和执行共用，
//! 走的是官方安装/升级的同一层（`install_detached`、`upgrade_upload`），不另写第二套。

use std::collections::BTreeMap;
use std::os::fd::AsFd;
use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject};
use serde_json::{Value, json};

use super::agent_cards::{install_ran, upgrade_ran};
use super::audit::{Decision, Record};
use super::cards;
use super::{Bus, Caller, ToolFailure, with_failure};
use crate::bot_activity::Guard;
use crate::files::share_fs::saving;
use crate::l10n;
use crate::plugin::check;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::{Manifest, Permission};
use crate::plugin::privileges::{self, CapabilityRisk};
use crate::plugin::store::{EnabledPermission, InstalledRecord};
use crate::plugin::{PluginPackage, package_files};
use wonton_protocol::{CardLine, PluginWriter, SecretValues};

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
    activity: Guard,
) -> CallToolResult {
    let value = Value::Object(args.clone());
    let id = args.get("id").and_then(Value::as_str);
    let dir = args.get("dir").and_then(Value::as_str);
    match (id, dir) {
        (Some(_), Some(_)) | (None, None) => bus.deny(
            caller,
            name,
            &value,
            &ToolFailure::new(
                "invalid_input",
                "invalid_input：id 和 dir 必须且只能给一个：官方索引里的用 id，自己写的用 dir",
            ),
        ),
        (Some(id), None) => official(bus, caller, name, &value, id, activity).await,
        (None, Some(dir)) => from_dir(bus, caller, name, &value, dir, activity).await,
    }
}

/// `id` 路径：官方索引里的插件，和以前一样经确认卡片安装。
async fn official(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    id: &str,
    activity: Guard,
) -> CallToolResult {
    let package = match super::core_tools::installable(bus, id).await {
        Ok(package) => package,
        Err(failure) => return bus.deny(caller, name, value, &failure),
    };
    let langs = bus.plugins.owner_languages();
    let inputs = match bus.plugins.secret_inputs(&package.manifest, None, &langs) {
        Ok(inputs) => inputs,
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &ToolFailure::new(e.kind(), e.to_string()),
            );
        }
    };
    let mut draft = cards::install(&caller.name, &package.manifest, inputs, &langs);
    draft.settings = match bus.plugins.card_settings(&package, None, &langs) {
        Ok(settings) => settings,
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &ToolFailure::new(e.kind(), e.to_string()),
            );
        }
    };
    match bus
        .plugins
        .retained_data(&package.manifest.id, &langs)
        .await
    {
        Ok(Some(notice)) => draft.lines.insert(1, notice),
        Ok(None) => {}
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &ToolFailure::new(
                    "internal_error",
                    format!("internal_error：查旧数据失败：{e}"),
                ),
            );
        }
    }
    let second = highest(&package.manifest).then(|| cards::highest_confirm(&package, &langs));
    let granted = package.manifest.permissions.clone();
    let langs = langs.clone();
    let id_for_run = package.manifest.id.clone();
    let run = move |bus: Arc<Bus>,
                    secrets: SecretValues,
                    settings: wonton_protocol::SettingValues| async move {
        install_ran(
            bus, id_for_run, package, granted, false, secrets, settings, None, langs,
        )
        .await
    };
    bus.confirmed_twice(
        caller,
        name,
        value.clone(),
        (draft, second, None, activity),
        run,
    )
    .await
}

/// 更新卡片顶上的一句话和要列的行：接手（#817）、权限变宽、有风险、每次都问，各说各的。
fn update_header(
    adopt: bool,
    widened: &[CardLine],
    risky: bool,
    requester: &str,
    package: &PluginPackage,
    langs: &Languages,
) -> (String, Vec<CardLine>) {
    if adopt {
        // 三件事要说清（#817）：换成 Bot 的版本、数据保留、以后没变宽不再问。权限变化照常列。
        let mut lines = vec![
            cards::line(l10n::text(langs, "chat-card-update-adopt-replace", &[])),
            cards::line(l10n::text(
                langs,
                "chat-card-agent-ask-again",
                &[("requester", requester.into())],
            )),
        ];
        lines.extend(widened.iter().cloned());
        (
            l10n::text(
                langs,
                "chat-card-update-header-adopt",
                &[("requester", requester.into())],
            ),
            lines,
        )
    } else if !widened.is_empty() {
        (
            l10n::text(langs, "chat-card-update-header-added", &[]),
            widened.to_vec(),
        )
    } else if risky {
        (
            l10n::text(langs, "chat-card-update-header-risky", &[]),
            cards::risky_lines(package, langs),
        )
    } else {
        (
            l10n::text(langs, "chat-card-update-header-ask-every", &[]),
            Vec::new(),
        )
    }
}

/// `dir` 路径：outbox 里的插件目录，校验、判定（安装还是更新、归谁写），然后卡片或直接更新。
async fn from_dir(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    dir: &str,
    activity: Guard,
) -> CallToolResult {
    let deny = |failure: ToolFailure| bus.deny(caller, name, value, &failure);
    let app_data = bus.plugins.app_data_dir(&caller.id);
    // 目录 fd 一路钉住：校验和装的都是这一眼看到的包，中途换成软链接也出不去。
    let opened = match saving::outbox_dir(&app_data, dir) {
        Ok((opened, _)) => opened,
        Err(failure) => return deny(ToolFailure::new(failure.kind.as_str(), failure.to_string())),
    };
    let langs = bus.plugins.owner_languages();
    // 只读一次：对这个包做检查，装的就是校验的这一份，中间改文件换不掉。
    let package = match PluginPackage::load_dir_at(opened.as_fd(), &langs) {
        Ok(package) => package,
        Err(e) => {
            return deny(ToolFailure::new(
                "invalid_input",
                format!("invalid_input：插件包不合法：{e}"),
            ));
        }
    };
    // 和 wonton__check_plugin 同一份校验：有 problems 就不发卡片，全部列出来。
    let problems =
        match check::check_package(&package, opened.as_fd(), false, bus.plugins.images()).await {
            Ok(report) => report.problems,
            Err(e) => vec![e],
        };
    if !problems.is_empty() {
        return deny(ToolFailure::new(
            "invalid_input",
            format!("invalid_input：校验没过，改完再装：{}", problems.join("；")),
        ));
    }
    // 和 id 路径一样，卡片之前就知道这台主机装不装得了。
    if let Err(e) = bus.plugins.fits_host(&package.manifest, &langs).await {
        return deny(ToolFailure::new(
            "invalid_input",
            format!("invalid_input：{e}"),
        ));
    }
    let id = package.manifest.id.clone();
    let record = match bus.plugins.installed_record(&id) {
        Ok(record) => record,
        Err(e) => {
            return deny(ToolFailure::new(
                "internal_error",
                format!("internal_error：读插件记录失败：{e}"),
            ));
        }
    };
    match record {
        None => first_install(bus, caller, name, value, package, &langs, activity).await,
        Some(record) => {
            update(
                bus,
                caller,
                name,
                value,
                package,
                (record, &langs, activity),
            )
            .await
        }
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
    activity: Guard,
) -> CallToolResult {
    let id = package.manifest.id.clone();
    // 之前还在等的同一份提交让位：主人不会面对两张都作数的卡。
    bus.confirmations
        .supersede_pending(&caller.id, &id, langs)
        .await;
    let inputs = match bus.plugins.secret_inputs(&package.manifest, None, langs) {
        Ok(inputs) => inputs,
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &ToolFailure::new(e.kind(), e.to_string()),
            );
        }
    };
    let mut draft = cards::agent_install(&caller.name, &package, inputs, langs);
    draft.settings = match bus.plugins.card_settings(&package, None, langs) {
        Ok(settings) => settings,
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &ToolFailure::new(e.kind(), e.to_string()),
            );
        }
    };
    match bus.plugins.retained_data(&id, langs).await {
        Ok(Some(notice)) => draft.lines.insert(1, notice),
        Ok(None) => {}
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &ToolFailure::new(
                    "internal_error",
                    format!("internal_error：查旧数据失败：{e}"),
                ),
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
    let langs = langs.clone();
    let run = move |bus: Arc<Bus>,
                    secrets: SecretValues,
                    settings: wonton_protocol::SettingValues| async move {
        install_ran(
            bus,
            id_for_run,
            package,
            granted,
            false,
            secrets,
            settings,
            Some(writer),
            langs,
        )
        .await
    };
    bus.confirmed_twice(
        caller,
        name,
        value.clone(),
        (draft, second, None, activity),
        run,
    )
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
        .upgrade_upload(
            &id,
            package,
            granted,
            record.enabled_permissions.clone(),
            record.lan_webui,
            &Default::default(),
            &bus.plugins.owner_languages(),
        )
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
            "note": "权限没变宽，没有打扰你，已写进审计",
        })),
        Err(e) => with_failure(
            &format!("更新 {} 失败（{}）：{}", id, e.kind(), e),
            e.kind(),
        ),
    }
}

async fn quiet_update_detached(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    package: PluginPackage,
    (record, granted, activity): (InstalledRecord, Vec<Permission>, Guard),
) -> CallToolResult {
    let (bus, caller, name, value) = (bus.clone(), caller.clone(), name.to_owned(), value.clone());
    tokio::spawn(async move {
        let result = quiet_update(&bus, &caller, &name, &value, package, &record, granted).await;
        drop(activity);
        result
    })
    .await
    .unwrap_or_else(|e| with_failure(&format!("更新流程意外中止：{e}"), "internal_error"))
}

/// 写审计；其余发卡片，最高级再多点一张。摘要和装着的相同报“没有变化”。
async fn update(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    package: PluginPackage,
    (record, langs, activity): (InstalledRecord, &Languages, Guard),
) -> CallToolResult {
    let deny = |failure: ToolFailure| bus.deny(caller, name, value, &failure);
    let id = package.manifest.id.clone();
    let owns = record
        .written_by
        .as_ref()
        .is_some_and(|w| w.plugin_id == caller.id.to_string());
    // 主人自己上传的插件（没有来源，也不归哪个 Bot）归主人：Agent 想接手得主人点头，更新
    // 一定发卡片，批下来后归属记成这个 Bot（#817）。官方、社区装的、别的 Bot 写的和系统
    // 插件照旧拒绝。
    let adopt = !owns
        && record.written_by.is_none()
        && record.origin.is_none()
        && !bus.plugins.is_system(&id);
    if !owns && !adopt {
        return deny(ToolFailure::new(
            "invalid_input",
            format!("invalid_input：{id} 已经装了，不是你写的插件，不能从这里更新"),
        ));
    }
    let installed_digest = package_files::digest(record.source.as_bytes(), &record.files);
    if installed_digest == package.digest() && !adopt {
        return deny(ToolFailure::new(
            "invalid_input",
            "invalid_input：没有变化：装着的就是这份（摘要相同）",
        ));
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
    // 新增的必填密钥没填过：静默更新会让启动失败，得发给主人填。
    let filled = bus.plugins.store().secrets().load(&id).ok().flatten();
    let needs_secrets = package.manifest.secrets.iter().any(|(name, spec)| {
        matches!(spec, crate::plugin::manifest::SecretSpec::Input(input) if input.required)
            && !filled
                .as_ref()
                .is_some_and(|secrets| secrets.contains_key(name))
    });
    if !adopt && widened.is_empty() && !risky && !record.ask_every_update && !needs_secrets {
        // 不打扰主人：直接更新，写审计（ADR-0033 的“更新，权限没变宽”）。之前还在等的
        // 同一份提交让位，不能让主人批一份已经被盖掉的。
        bus.confirmations
            .supersede_pending(&caller.id, &id, langs)
            .await;
        return quiet_update_detached(
            bus,
            caller,
            name,
            value,
            package,
            (record, granted, activity),
        )
        .await;
    }
    let enabled = record.enabled_permissions.clone();
    let lan_webui = record.lan_webui;
    card_update(
        bus,
        caller,
        name,
        value,
        package,
        (record, langs, activity),
        (adopt, risky, widened),
        granted,
        enabled,
        lan_webui,
        filled,
    )
    .await
}

/// 发卡片的更新：接手（#817）的抬头不同，批下来后把归属记成这个 Bot。
#[allow(clippy::too_many_arguments)] // 卡片和执行要用的几个值，打包反而看不清哪来的。
async fn card_update(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    value: &Value,
    package: PluginPackage,
    (record, langs, activity): (InstalledRecord, &Languages, Guard),
    (adopt, risky, widened): (bool, bool, Vec<CardLine>),
    granted: Vec<Permission>,
    enabled: Vec<EnabledPermission>,
    lan_webui: bool,
    filled: Option<BTreeMap<String, Vec<u8>>>,
) -> CallToolResult {
    let id = package.manifest.id.clone();
    let version = package.manifest.version.clone();
    if adopt {
        // 冷却（#817）：主人回绝或没理之后不再刷卡，让 Bot 过会儿再来。
        if let Some(wait) = bus.adopt_cooldown(&caller.id, &id) {
            let minutes = wait.as_secs().div_ceil(60);
            return bus.deny(
                caller,
                name,
                value,
                &ToolFailure::new(
                    "invalid_input",
                    format!("invalid_input：接手刚被回绝，{minutes} 分钟内不会再问"),
                ),
            );
        }
        bus.note_adopt_attempt(&caller.id, &id);
    }
    // 发卡片前让位：同一份插件只留一张等确认的卡。
    bus.confirmations
        .supersede_pending(&caller.id, &id, langs)
        .await;
    let (header, lines) = update_header(adopt, &widened, risky, &caller.name, &package, langs);
    let inputs = match bus
        .plugins
        .secret_inputs(&package.manifest, filled.as_ref(), langs)
    {
        Ok(inputs) => inputs,
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &ToolFailure::new(e.kind(), e.to_string()),
            );
        }
    };
    let mut draft = cards::agent_update(
        &caller.name,
        &package,
        &header,
        &lines,
        &record.version,
        inputs,
        langs,
    );
    draft.settings = match bus.plugins.card_settings(&package, Some(&record), langs) {
        Ok(settings) => settings,
        Err(e) => {
            return bus.deny(
                caller,
                name,
                value,
                &ToolFailure::new(e.kind(), e.to_string()),
            );
        }
    };
    let second = highest(&package.manifest).then(|| cards::highest_confirm(&package, langs));
    let id_for_run = id;
    let writer = adopt.then(|| PluginWriter {
        plugin_id: caller.id.to_string(),
        title: caller.name.clone(),
    });
    let run =
        move |bus: Arc<Bus>, secrets: SecretValues, settings: wonton_protocol::SettingValues| {
            upgrade_ran(
                bus, id_for_run, version, package, granted, enabled, lan_webui, secrets, settings,
                writer,
            )
        };
    bus.confirmed_twice(
        caller,
        name,
        value.clone(),
        (draft, second, None, activity),
        run,
    )
    .await
}
