//! 工具结果里的文件（ADR-0020 的“Bot”，插件规范的“工具结果里的文件”）：这是“总线不解释工具结果”（ADR-0016）的唯一例外。结构化的引用换
//! 成编号和一段文字，前几张图片附上预览，其余内容块原样交给 Agent。不扫文字内容：文字里提到的路径不会变成编号，提示注入也就不能借此登记
//! 文件。

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{CallToolResult, ContentBlock, MetaObject, Resource, ResourceContents};
use tokio::time::Instant;
use wonton_protocol::FileFailure;

use super::audit::AuditedFile;
use super::{Bus, Caller};
use crate::files::content::{can_thumbnail, declared_type, served_type};
use crate::files::record::FileRecord;
use crate::files::share_fs::ShareRef;
use crate::files::{BotRef, Failure, Files};
use crate::plugin::PluginId;

/// 一个结果最多登记这么多个文件，每个占一行上下文。
const MAX_FILES: usize = 100;
/// 一个结果最多附这么多张预览。
const MAX_PREVIEWS: usize = 4;
/// 登记要开文件、写记录，共享文件夹卡住时后面的引用不能一个个等。
const REGISTER_BUDGET: Duration = Duration::from_secs(30);
/// 只有核心读的 `_meta`，交给 Agent 前去掉。
const CORE_META_PREFIX: &str = "app.wonton/";
const INTERNAL_URL_META: &str = "app.wonton/internal_url";

/// 排好顺序的输出：普通的内容块，或者登记好的文件。
enum Part {
    Block(ContentBlock),
    File(FileRecord),
}

/// 换掉结果里的引用，返回给 Agent 的结果和登记的文件。
pub(super) async fn rewrite(
    bus: &Arc<Bus>,
    caller: &Caller,
    plugin: &PluginId,
    mut result: CallToolResult,
) -> (CallToolResult, Vec<AuditedFile>) {
    strip_meta(&mut result.meta);
    let target = bus
        .files()
        .and_then(|files| Ok((files.clone(), bus.bot_ref(caller, plugin)?)));
    let deadline = Instant::now() + REGISTER_BUDGET;
    let (mut parts, mut seen) = (Vec::with_capacity(result.content.len()), 0);
    let mut refusals = Vec::new();
    for block in std::mem::take(&mut result.content) {
        let link = match block {
            ContentBlock::ResourceLink(link) if !is_web(&link.uri) => link,
            ContentBlock::ResourceLink(mut link) => {
                strip_meta(&mut link.meta);
                parts.push(Part::Block(ContentBlock::ResourceLink(link)));
                continue;
            }
            mut other => {
                strip_block_meta(&mut other);
                parts.push(Part::Block(other));
                continue;
            }
        };
        seen += 1;
        if seen > MAX_FILES {
            continue;
        }
        let registered = match &target {
            Ok((files, bot)) => {
                tokio::time::timeout_at(deadline, register(files, bot, plugin, &link)).await
            }
            Err(failure) => Ok(Err(failure.clone())),
        };
        let failure = match registered {
            Ok(Ok(record)) => {
                parts.push(Part::File(record));
                continue;
            }
            Ok(Err(failure)) => failure,
            Err(_) => Failure::new(FileFailure::ServiceUnavailable, "登记超时，稍后再试"),
        };
        refusals.push(AuditedFile::refused(plugin, &link.uri, failure.kind));
        parts.push(Part::Block(ContentBlock::text(refused(&link, &failure))));
    }
    let previews = previews(target.as_ref().ok().map(|(files, _)| files), &parts).await;
    let mut audited = Vec::new();
    let (mut content, mut previewed) = (Vec::with_capacity(parts.len()), false);
    for (part, preview) in parts.into_iter().zip(previews) {
        match part {
            Part::Block(block) => content.push(block),
            Part::File(record) => {
                audited.push(AuditedFile::of(&record));
                content.push(ContentBlock::text(file_line(&record)));
                if let Some(preview) = preview {
                    previewed = true;
                    content.push(ContentBlock::image(preview, "image/jpeg"));
                }
            }
        }
    }
    if seen > MAX_FILES {
        content.push(ContentBlock::text(format!(
            "还有 {} 个文件没有列出：一个结果最多 {MAX_FILES} 个，缩小范围再找。",
            seen - MAX_FILES
        )));
    }
    if !audited.is_empty() {
        content.push(ContentBlock::text(hint(previewed)));
    }
    audited.extend(refusals);
    result.content = content;
    (result, audited)
}

async fn register(
    files: &Arc<Files>,
    bot: &BotRef,
    plugin: &PluginId,
    link: &Resource,
) -> Result<FileRecord, Failure> {
    let mime_type = link.mime_type.as_deref();
    if ShareRef::is_reference(&link.uri) {
        let reference = ShareRef::parse(&link.uri)
            .map_err(|why| Failure::new(FileFailure::PermissionDenied, why))?;
        let name = Some(link.name.as_str()).filter(|name| !name.is_empty());
        return files.register_share(bot, reference, name, mime_type).await;
    }
    let internal = link
        .meta
        .as_ref()
        .and_then(|meta| meta.0.get(INTERNAL_URL_META))
        .and_then(|url| url.as_str());
    files
        .register_resource(
            bot, plugin, &link.uri, internal, &link.name, mime_type, link.size,
        )
        .await
}

/// 每张图片的预览，和 `parts` 一一对应，前 [`MAX_PREVIEWS`] 张之外都是空的。解不了的也占名额、不附，文字照常。
async fn previews(files: Option<&Arc<Files>>, parts: &[Part]) -> Vec<Option<String>> {
    let mut left = MAX_PREVIEWS;
    let jobs = parts.iter().map(|part| {
        let wanted = match part {
            Part::File(record) if left > 0 && viewable(record) => {
                left -= 1;
                files.map(|files| (files, record))
            }
            _ => None,
        };
        async move {
            let (files, record) = wanted?;
            // 插件已经回答了，Agent 和确认卡片不该等预览：单张的读取和解码期限加起来有两分钟。到时没做完的不附，想看再调 view_file。
            // 只是不等了，读取留在后台：丢掉 `resources/read` 的 future 时插件收不到取消，还在读，名额和“占着会话”的记号却提前没了，
            // 同时读的就超过名额，串行的插件还会被误判超时。后台的这次仍受名额和读取期限的约束。
            let job = tokio::spawn({
                let (files, record) = (files.clone(), record.clone());
                async move { files.preview(&record).await }
            });
            match tokio::time::timeout(files.preview_budget, job).await {
                Ok(Ok(Ok(jpeg))) => Some(STANDARD.encode(jpeg)),
                Ok(Ok(Err(e))) => {
                    tracing::info!(file = %record.id, error = %e, "这个文件不附预览");
                    None
                }
                Ok(Err(e)) => {
                    tracing::error!(file = %record.id, error = %e, "做预览的任务异常退出");
                    None
                }
                Err(_) => {
                    tracing::info!(file = %record.id, "预览没在期限内做完，不附");
                    None
                }
            }
        }
    });
    futures_util::future::join_all(jobs).await
}

fn viewable(record: &FileRecord) -> bool {
    can_thumbnail(&record.name, record.mime_type.as_deref())
}

fn is_web(uri: &str) -> bool {
    let scheme = uri.split_once(':').map_or("", |(scheme, _)| scheme);
    scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
}

/// 这个文件的一行文字：编号、文件名、类型、大小。文件名经过 `sanitize_name`，插件给的类型经过 [`declared_type`]，不会多出一行。
fn file_line(record: &FileRecord) -> String {
    let declared = record.mime_type.as_deref();
    let mime = served_type(&record.name, declared)
        .or_else(|| declared.and_then(declared_type))
        .unwrap_or("类型未知");
    let size = record
        .registered
        .map(|snapshot| snapshot.size)
        .or(record.declared_size)
        .map_or_else(|| "大小未知".to_owned(), human_size);
    format!("文件 {}（{mime}，{size}），编号 {}", record.name, record.id)
}

/// 只说一次能调的工具：搜索结果可能有很多行。
fn hint(previewed: bool) -> String {
    let preview = if previewed {
        "附的图片只是缩小的预览，不是原文件。"
    } else {
        ""
    };
    format!(
        "{preview}看图片、读文本文件内容调 wonton__view_file，发给主人调 wonton__share_file（参数都是 file_id），把返回的链接原样放进回复。"
    )
}

/// 登记不了时 Agent 看到的说明。不写引用本身：内网地址不能给 Agent。
fn refused(link: &Resource, failure: &Failure) -> String {
    // 说明里可能带着引用里的共享文件夹名字，它是插件给的。
    let why: String = if ShareRef::is_reference(&link.uri) {
        failure
            .message
            .chars()
            .filter(|c| !c.is_control())
            .collect()
    } else {
        "插件给的资源引用用不了".to_owned()
    };
    format!(
        "文件 {} 登记不了（{}）：{why}",
        crate::files::share_fs::sanitize_name(&link.name),
        failure.kind.as_str()
    )
}

pub(super) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn strip_block_meta(block: &mut ContentBlock) {
    match block {
        ContentBlock::Text(text) => strip_meta(&mut text.meta),
        ContentBlock::Image(image) => strip_meta(&mut image.meta),
        ContentBlock::Audio(audio) => strip_meta(&mut audio.meta),
        ContentBlock::ResourceLink(link) => strip_meta(&mut link.meta),
        ContentBlock::Resource(embedded) => {
            strip_meta(&mut embedded.meta);
            match &mut embedded.resource {
                ResourceContents::TextResourceContents { meta, .. }
                | ResourceContents::BlobResourceContents { meta, .. } => strip_meta(meta),
                _ => {}
            }
        }
        _ => {}
    }
}

/// 去掉只有核心读的键，什么都不剩时连 `_meta` 也不留。
fn strip_meta(meta: &mut Option<MetaObject>) {
    if let Some(object) = meta {
        object.0.retain(|key, _| !key.starts_with(CORE_META_PREFIX));
        if object.0.is_empty() {
            *meta = None;
        }
    }
}
