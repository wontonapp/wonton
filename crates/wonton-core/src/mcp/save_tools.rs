//! Bot 把文件存进共享文件夹（ADR-0027）：目标在已授权的路径里就放，否则发卡片，同意了连路径一起记下。

use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde_json::{Value, json};
use wonton_protocol::{AiLevel, Answer, ConfirmationState, FileFailure};

use super::audience::Subject;
use super::audit::{Basis, Decision, Record};
use super::core_tools::tool;
use super::file_tools::decision;
use super::{Bus, Caller, Outcome, ToolFailure, cards, naming, with_failure};
use crate::bot_activity::Guard;
use crate::files::Failure;
use crate::files::share_fs::saving::{SaveTarget, Staged};
use crate::l10n;
use crate::plugin::save_grants::covers;

pub(super) const LIST: &str = "list_shares";
pub(super) const SAVE: &str = "save_file";
/// 一次最多存这么多个文件。
const MAX_FILES: usize = 100;

pub(super) fn definitions() -> Vec<Tool> {
    vec![
        tool(
            LIST,
            "列出主人的共享文件夹的名字，wonton__save_file 能存进这些文件夹。",
            json!({"type": "object", "properties": {}}),
            true,
        ),
        tool(
            SAVE,
            "把文件存进主人的共享文件夹。先把文件放进你的 outbox 目录，再调用。只能新增：同名时自动加序号，不覆盖。存好后 outbox 里的原文件会被删掉。\
             第一次存进一个文件夹要主人确认：调用前先用一句话告诉主人，确认卡片会发到 Wonton App 里的“Wonton”私聊。调用会一直等到主人回答或者确认过期\
             （10 分钟）。主人允许后，这个文件夹以后不用再问。存没存上以返回的结果为准，照实转告；返回每个文件最后的路径。",
            json!({"type": "object", "properties": {
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 1,
                    "maxItems": MAX_FILES,
                    "description": "outbox 里的相对路径，如 song.mp3 或 album/01.mp3",
                },
                "share": {"type": "string", "description": "共享文件夹的名字，如 music"},
                "dir": {"type": "string", "description": "共享文件夹里的子目录，如 周杰伦/七里香，没有的会建出来；不写就存在共享文件夹的最上层"},
            }, "required": ["files", "share"]}),
            false,
        ),
    ]
}

pub(super) async fn list(bus: &Bus) -> Result<Value, ToolFailure> {
    let files = bus
        .files()
        .map_err(|failure| ToolFailure::new(failure.kind.as_str(), failure.to_string()))?;
    let shares = files
        .share_names()
        .await
        .map_err(|failure| ToolFailure::new(failure.kind.as_str(), failure.to_string()))?;
    Ok(json!({ "shares": shares }))
}

/// 放在后台任务里：暂存要有人收拾，确认过的调用不跟着 Agent 走（ADR-0016）。
pub(super) async fn save(
    bus: &Arc<Bus>,
    caller: &Caller,
    args: JsonObject,
    activity: Guard,
) -> CallToolResult {
    // 失败即停：封顶“只读”的存不了共享文件夹（ADR-0088 第 5 节）。
    if bus.ai_cap(caller) == AiLevel::ReadOnly {
        let name = naming::join(naming::CORE, SAVE);
        let value = Value::Object(args.clone());
        drop(activity);
        return bus.deny(
            caller,
            &name,
            &value,
            &ToolFailure::new(
                "permission_denied",
                "这次的 AI 动手级别是只读，不能把文件存进共享文件夹，没有执行",
            ),
        );
    }
    let (bus, caller) = (bus.clone(), caller.clone());
    tokio::spawn(async move { run(&bus, &caller, args, activity).await })
        .await
        .unwrap_or_else(|e| with_failure(&format!("存文件的流程意外中止：{e}"), "internal_error"))
}

async fn run(bus: &Arc<Bus>, caller: &Caller, args: JsonObject, activity: Guard) -> CallToolResult {
    let name = naming::join(naming::CORE, SAVE);
    let value = Value::Object(args.clone());
    let (target, staged) = match stage(bus, caller, &args).await {
        Ok(ready) => ready,
        Err(failure) => {
            bus.audit.record(&Record {
                caller: caller.id.as_str(),
                tool: &name,
                arguments: &value,
                decision: decision(&failure),
                basis: None,
                outcome: Some(failure.kind.as_str()),
                files: &[],
            });
            return with_failure(&failure.to_string(), failure.kind.as_str());
        }
    };
    let folder = target.folder();
    let grants = bus.plugins.save_grants(&caller.id);
    if let Some(matched) = grants.iter().find(|granted| covers(granted, &folder)) {
        let summary = summary(&target, &staged, "matched", matched);
        let (outcome, audited) = place(bus, staged, target, summary, None).await;
        bus.record(caller, &name, &audited, Decision::Allowed, None, &outcome);
        return outcome.result;
    }
    let langs = bus.plugins.owner_languages();
    let draft = cards::save_file(&caller.name, &staged.files, &target, &langs);
    drop(activity);
    // 封顶“删除要管理员批”时存共享文件夹的卡片发管理员（ADR-0088 第 5 节）；其余照 ADR-0027 发主人。
    let ticket = match if bus.ai_cap(caller) == AiLevel::AdminApprovesDeletes {
        bus.ask_for(caller, Subject::ToolDelete, draft).await
    } else {
        bus.confirmations.ask(&caller.id, draft).await
    } {
        Ok(ticket) => ticket,
        Err(refusal) => return bus.refused(caller, &name, &value, &refusal, None),
    };
    let activity = bus.plugins.activity.start_tool(&caller.id);
    let granted = match ticket.answer {
        Answer::ApproveWider => target.share.clone(),
        _ => folder,
    };
    let note = bus
        .plugins
        .add_save_grant(&caller.id, &granted)
        .err()
        .map(|e| {
            tracing::error!(plugin = %caller.id, error = %e, "记存文件的授权失败");
            l10n::text(
                &langs,
                "chat-save-note-grant",
                &[("error", e.to_string().into())],
            )
        });
    let summary = summary(&target, &staged, "granted", &granted);
    let (outcome, audited) = place(bus, staged, target, summary, note).await;
    drop(activity);
    bus.record(
        caller,
        &name,
        &audited,
        Decision::Approved,
        Some(Basis::Card.into()),
        &outcome,
    );
    bus.confirmations
        .finish(ticket, outcome.state.clone())
        .await;
    outcome.result
}

/// 不需要主人确认的那部分：查参数，把文件复制进暂存。
async fn stage(
    bus: &Bus,
    caller: &Caller,
    args: &JsonObject,
) -> Result<(SaveTarget, Staged), Failure> {
    let invalid = |why: &str| Failure::new(FileFailure::InvalidInput, why);
    let paths: Vec<String> = args
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| {
            files
                .iter()
                .map(|file| file.as_str().map(str::to_owned))
                .collect()
        })
        .ok_or_else(|| invalid("files 要是 outbox 里的相对路径的列表"))?;
    if paths.is_empty() || paths.len() > MAX_FILES {
        return Err(invalid("files 要有 1 到 100 个"));
    }
    let share = args
        .get("share")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("缺少参数 share"))?;
    let dir = args.get("dir").and_then(Value::as_str);
    let target = SaveTarget::parse(share, dir)?;
    let app_data = bus.plugins.app_data_dir(&caller.id);
    // Agent 替主人存文件：按主人最近一次的语言。
    let langs = bus.plugins.owner_languages();
    let staged = bus
        .files()?
        .stage_save(app_data, paths, &target, &langs)
        .await?;
    Ok((target, staged))
}

/// 审计里的这次保存：目标、文件、经哪条授权。
fn summary(target: &SaveTarget, staged: &Staged, how: &str, path: &str) -> Value {
    let files: Vec<Value> = staged
        .files
        .iter()
        .map(|file| json!({"name": file.name, "size": file.size, "sha256": file.sha256}))
        .collect();
    json!({"share": target.share, "dir": target.dirs.join("/"), "files": files, how: path})
}

async fn place(
    bus: &Bus,
    staged: Staged,
    target: SaveTarget,
    mut summary: Value,
    note: Option<String>,
) -> (Outcome, Value) {
    let folder = target.folder();
    let langs = bus.plugins.owner_languages();
    let placed = match bus.files() {
        Ok(files) => {
            let langs = bus.plugins.owner_languages();
            files.place_saved(staged, target, &langs).await
        }
        Err(failure) => Err(failure),
    };
    let placed = match placed {
        Ok(placed) => placed,
        Err(failure) => {
            return (
                Outcome::failed(failure.kind.as_str(), failure.to_string()),
                summary,
            );
        }
    };
    summary["saved"] = json!(placed.saved);
    if let Some(failed) = placed.failed {
        let failure = Failure::from(failed);
        let message = if placed.saved.is_empty() {
            failure.to_string()
        } else {
            l10n::text(
                &langs,
                "chat-save-partial",
                &[
                    ("saved", l10n::list(&langs, &placed.saved).into()),
                    ("failure", failure.to_string().into()),
                ],
            )
        };
        return (Outcome::failed(failure.kind.as_str(), message), summary);
    }
    let mut result = json!({"saved": placed.saved});
    if let Some(note) = &note {
        result["note"] = json!(note);
    }
    let message = match note {
        Some(note) => l10n::text(
            &langs,
            "chat-save-done-note",
            &[("folder", folder.as_str().into()), ("note", note.into())],
        ),
        None => l10n::text(
            &langs,
            "chat-save-done",
            &[("folder", folder.as_str().into())],
        ),
    };
    let outcome = Outcome {
        result: CallToolResult::success(vec![ContentBlock::text(result.to_string())]),
        state: ConfirmationState::Succeeded { message },
        files: Vec::new(),
    };
    (outcome, summary)
}

impl Bus {
    /// 主人撤销一条授权，只拦后面的调用。
    pub fn revoke_save_grant(
        &self,
        id: &crate::plugin::PluginId,
        folder: &str,
    ) -> Result<(), crate::plugin::store::StoreError> {
        self.plugins.remove_save_grant(id, folder)?;
        self.audit.record(&Record {
            caller: id.as_str(),
            tool: "wonton__revoke_save_grant",
            arguments: &json!({"folder": folder}),
            decision: Decision::Approved,
            basis: None,
            outcome: Some("ok"),
            files: &[],
        });
        Ok(())
    }
}
