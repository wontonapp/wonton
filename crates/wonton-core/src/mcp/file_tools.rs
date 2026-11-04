//! Bot 的两个文件工具（ADR-0020 的“Bot”）：只收编号，不接受路径或 URL，提示注入发不出 `/etc/passwd`，也发不出 Agent 没见过的文件。错误
//! 都是 `isError: true`，文字开头是类别。

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde_json::{Value, json};
use wonton_protocol::{FileFailure, FileId};

use super::audit::{AuditedFile, Decision, Record};
use super::core_tools::tool;
use super::{Bus, Caller, error, naming};
use crate::files::Failure;
use crate::files::record::FileRecord;

pub(super) const VIEW: &str = "view_file";
pub(super) const SHARE: &str = "share_file";

pub(super) fn definitions() -> Vec<Tool> {
    let input = json!({"type": "object", "properties": {
        "file_id": {"type": "string", "description": "工具结果里写的文件编号"},
    }, "required": ["file_id"]});
    vec![
        tool(
            VIEW,
            "看一张图片：返回缩小的预览。参数是工具结果里写的文件编号。不是图片时返回 not_viewable；文件登记后改过时返回 \
             file_changed，要重新找。",
            input.clone(),
            true,
        ),
        tool(
            SHARE,
            "把文件发给主人：只有主人能看，文件不离开主人的设备，不用确认。参数是工具结果里写的文件编号。返回一个链接，把它原样放进回复，\
             主人的 Wonton App 会显示成文件卡片。不要改动链接，也不要用别的办法发这个文件。",
            input,
            false,
        ),
    ]
}

pub(super) async fn call(
    bus: &Arc<Bus>,
    caller: &Caller,
    tool: &str,
    args: JsonObject,
) -> CallToolResult {
    let name = naming::join(naming::CORE, tool);
    let value = Value::Object(args.clone());
    let done = run(bus, caller, tool, &args).await;
    let (outcome, files) = match &done {
        Ok((_, record)) => ("ok", vec![AuditedFile::of(record)]),
        Err(failure) => (failure.kind.as_str(), Vec::new()),
    };
    bus.audit.record(&Record {
        caller: caller.id.as_str(),
        tool: &name,
        arguments: &value,
        decision: done.as_ref().err().map_or(Decision::Allowed, decision),
        outcome: Some(outcome),
        files: &files,
    });
    done.map_or_else(|failure| error(&failure.to_string()), |(result, _)| result)
}

/// 文件不在范围里、没有授权、参数不对才算拒绝。文件没了、解码忙是执行时出的问题，类别写在 `outcome` 里。
pub(super) fn decision(failure: &Failure) -> Decision {
    match failure.kind {
        FileFailure::Unauthenticated
        | FileFailure::NotShared
        | FileFailure::FileUnknown
        | FileFailure::FileExpired
        | FileFailure::PermissionDenied
        | FileFailure::InvalidInput => Decision::Denied,
        FileFailure::FileGone
        | FileFailure::FileChanged
        | FileFailure::NotViewable
        | FileFailure::StorageFull
        | FileFailure::PluginUnavailable
        | FileFailure::UpstreamError
        | FileFailure::ResourceTooLarge
        | FileFailure::ServiceUnavailable
        | FileFailure::Internal
        | FileFailure::TransportUnavailable
        | FileFailure::Protocol
        | FileFailure::LocalSourceUnavailable
        | FileFailure::LocalPermissionDenied
        | FileFailure::LocalStorageFull
        | FileFailure::LocalIo
        | FileFailure::UnsupportedFormat
        | FileFailure::Other => Decision::Allowed,
    }
}

async fn run(
    bus: &Arc<Bus>,
    caller: &Caller,
    tool: &str,
    args: &JsonObject,
) -> Result<(CallToolResult, FileRecord), Failure> {
    let id = file_id(args)?;
    let files = bus.files()?;
    let bot = bus.bot_name(caller)?;
    let record = files.for_bot(&id, &bot).await?;
    if tool == VIEW {
        let jpeg = files.preview(&record).await?;
        let content = vec![
            ContentBlock::text(format!(
                "{} 的预览：缩小的图片，不是原文件。发给主人调 wonton__share_file。",
                record.name
            )),
            ContentBlock::image(STANDARD.encode(jpeg), "image/jpeg"),
        ];
        return Ok((CallToolResult::success(content), record));
    }
    let record = files.share_with_owner(&id, &bot).await?;
    let text = format!(
        "已分享给主人。把下面这个链接原样放进回复，主人会看到文件卡片：\n{}",
        record.id.link()
    );
    let content = vec![ContentBlock::text(text)];
    Ok((CallToolResult::success(content), record))
}

/// 编号写得不对，和没有这个编号是一回事：模型抄错了字符，该让它知道这个编号用不了。
fn file_id(args: &JsonObject) -> Result<FileId, Failure> {
    let id = args
        .get("file_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::new(FileFailure::InvalidInput, "缺少参数 file_id"))?;
    id.parse()
        .map_err(|_| Failure::new(FileFailure::FileUnknown, "没有这个文件"))
}
