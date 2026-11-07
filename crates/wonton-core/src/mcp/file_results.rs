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
use crate::plugin::languages::Languages;

/// 一个结果最多登记这么多个文件，每个占一行上下文。
const MAX_FILES: usize = 100;
/// 一个结果最多附这么多张预览。
const MAX_PREVIEWS: usize = 4;
/// 登记要开文件、写记录，共享文件夹卡住时后面的引用不能一个个等。
const REGISTER_BUDGET: Duration = Duration::from_secs(30);
/// 只有核心读的 `_meta`，交给 Agent 前去掉。
const CORE_META_PREFIX: &str = "app.wonton/";
const INTERNAL_URL_META: &str = "app.wonton/internal_url";
/// 单个二进制块（image、audio、嵌入资源的 blob）解码后最多这么多：超过的整块换成说明（插件规范的“工具结果里的文件”）。
/// 整个响应的上限是另一回事，在 upstream.rs。
const MAX_BLOCK_BYTES: usize = 1 << 20;

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
    let langs = bus.plugins.owner_languages();
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
                parts.push(Part::Block(checked_block(other)));
                continue;
            }
        };
        seen += 1;
        if seen > MAX_FILES {
            continue;
        }
        let registered = match &target {
            Ok((files, bot)) => {
                tokio::time::timeout_at(deadline, register(files, bot, plugin, &link, &langs)).await
            }
            Err(failure) => Ok(Err(failure.clone())),
        };
        let failure = match registered {
            Ok(Ok(record)) => {
                parts.push(Part::File(record));
                continue;
            }
            Ok(Err(failure)) => failure,
            Err(_) => Failure::new(
                FileFailure::ServiceUnavailable,
                "registration timed out; try again",
            ),
        };
        refusals.push(AuditedFile::refused(plugin, &link.uri, failure.kind));
        parts.push(Part::Block(ContentBlock::text(refused(&link, &failure))));
    }
    let previews = previews(target.as_ref().ok().map(|(files, _)| files), &parts, &langs).await;
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
            "{} more files not listed: one result holds at most {MAX_FILES}; narrow the search.",
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
    langs: &Languages,
) -> Result<FileRecord, Failure> {
    let mime_type = link.mime_type.as_deref();
    if ShareRef::is_reference(&link.uri) {
        let reference = ShareRef::parse(&link.uri)
            .map_err(|why| Failure::new(FileFailure::PermissionDenied, why))?;
        let name = Some(link.name.as_str()).filter(|name| !name.is_empty());
        return files
            .register_share(bot, reference, name, mime_type, langs)
            .await;
    }
    let internal = link
        .meta
        .as_ref()
        .and_then(|meta| meta.0.get(INTERNAL_URL_META))
        .and_then(|url| url.as_str());
    files
        .register_resource(
            bot, plugin, &link.uri, internal, &link.name, mime_type, link.size, langs,
        )
        .await
}

/// 每张图片的预览，和 `parts` 一一对应，前 [`MAX_PREVIEWS`] 张之外都是空的。解不了的也占名额、不附，文字照常。
async fn previews(
    files: Option<&Arc<Files>>,
    parts: &[Part],
    langs: &Languages,
) -> Vec<Option<String>> {
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
                let langs = langs.clone();
                async move { files.preview(&record, &langs).await }
            });
            match tokio::time::timeout(files.preview_budget, job).await {
                Ok(Ok(Ok(jpeg))) => Some(STANDARD.encode(jpeg)),
                Ok(Ok(Err(e))) => {
                    tracing::info!(file = %record.id, error = %e, "no preview attached for this file");
                    None
                }
                Ok(Err(e)) => {
                    tracing::error!(file = %record.id, error = %e, "preview task panicked");
                    None
                }
                Err(_) => {
                    tracing::info!(file = %record.id, "preview didn't finish in time; not attached");
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
        .unwrap_or("unknown type");
    let size = record
        .registered
        .map(|snapshot| snapshot.size)
        .or(record.declared_size)
        .map_or_else(|| "unknown size".to_owned(), human_size);
    format!("file {} ({mime}, {size}), id {}", record.name, record.id)
}

/// 只说一次能调的工具：搜索结果可能有很多行。
fn hint(previewed: bool) -> String {
    let preview = if previewed {
        "The attached image is a downscaled preview, not the original file. "
    } else {
        ""
    };
    format!(
        "{preview}Use wonton__view_file to view images or read text file contents, and wonton__share_file to send a file to the owner (both take file_id); put the returned link into your reply as-is."
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
        "the resource reference from the app can't be used".to_owned()
    };
    format!(
        "can't register file {} ({}): {why}",
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

/// 二进制块（image、audio、嵌入资源的 blob）超过单块上限或 base64 不合法时，整块换成说明；其余原样过（插件规范的
/// “工具结果里的文件”）。不截断，同一结果里的别的块不受影响。
fn checked_block(block: ContentBlock) -> ContentBlock {
    let (kind, data) = match &block {
        ContentBlock::Image(image) => ("image block", image.data.as_str()),
        ContentBlock::Audio(audio) => ("audio block", audio.data.as_str()),
        ContentBlock::Resource(embedded) => match &embedded.resource {
            ResourceContents::BlobResourceContents { blob, .. } => {
                ("blob in a resource block", blob.as_str())
            }
            _ => return block,
        },
        _ => return block,
    };
    let invalid = || {
        ContentBlock::text(format!(
            "the {kind} from the app is not valid base64; its content is not included."
        ))
    };
    // STANDARD 的合法 base64 长度必是 4 的倍数，不用解码就能先排除一段。大小按 base64 长度估，不为判断大小把几十
    // MiB 解码一遍再丢；上限以内的才解码核对，顺便查出长度对得上但内容不合规矩的。
    if data.len() % 4 != 0 {
        return invalid();
    }
    let estimated =
        (data.len() / 4 * 3).saturating_sub(data.len() - data.trim_end_matches('=').len());
    if estimated > MAX_BLOCK_BYTES {
        return ContentBlock::text(format!(
            "the {kind} from the app decodes to {}, over the single-block limit of {}; its content is not included. \
             To hand over large files, the app must return a resource_link reference (plugin spec, \"Files in tool results\") \
             so the agent can view and share them.",
            human_size(estimated as u64),
            human_size(MAX_BLOCK_BYTES as u64),
        ));
    }
    if STANDARD.decode(data).is_err() {
        return invalid();
    }
    block
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
