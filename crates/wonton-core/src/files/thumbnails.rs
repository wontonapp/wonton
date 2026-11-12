//! 缩略图和文本预览的读取：名额、期限和解码（[`Files::thumbnail`]、[`Files::preview`]、[`Files::text_preview`] 共用）。

use std::io::Read;
use std::time::Duration;

use tokio::sync::OwnedSemaphorePermit;
use wonton_protocol::FileFailure;

use super::content::can_thumbnail;
use super::record::{FileRecord, Origin};
use super::resource::{self, RemoteRequest};
use super::share_fs::ShareRef;
use super::slots::PluginSlot;
use super::{Content, Failure, Files, MAX_RESOURCE_BYTES, MAX_TEXT_PREVIEW_BYTES, share_failure};
use crate::l10n;
use crate::plugin::image_worker::ImageError;
use crate::plugin::images::MAX_THUMBNAIL_INPUT;
use crate::plugin::languages::Languages;

/// 同时最多读这么多张本地的原图，每张最多 [`MAX_THUMBNAIL_INPUT`]。插件的原图另有名额（[`slots`]）：存储卡住时插件的缩略图
/// 照常，插件慢时本地的也照常。和 [`ImageWorker`] 的名额分开：读可能要等慢插件，不能占着解码的名额，挡住别的图片处理（比如插件商店
/// 的图标）。
pub(super) const MAX_THUMBNAIL_READS: usize = 4;
/// 读完的原图最多这么多张在内存里等解码或者正在解码。读的名额读完就还：解码慢不是来源的问题。满了不等，读完也白读，但内存有上限。
pub(super) const MAX_THUMBNAIL_DECODES: usize = 8;
/// 等解码加解码最多这么久，比 [`ImageWorker`] 自己等子进程的 10 秒长：解码的毛病由它报。
pub(super) const THUMBNAIL_DECODE_TIMEOUT: Duration = Duration::from_secs(30);

/// [`Files::read_head`] 的中间结果；名额跟着读走，读完才还。
pub(super) struct HeadRead {
    pub(super) bytes: Vec<u8>,
    pub(super) total: Option<u64>,
    pub(super) truncated: bool,
    pub(super) slot: ThumbnailSlot,
}

impl Files {
    /// 开头 [`MAX_TEXT_PREVIEW_BYTES`] 字节和截断信息。读 `上限+1` 个字节：读满就是截断，内容截到上限。
    /// 本地文件只读这么多；插件的流攒够就停，不往下拉。
    pub(super) async fn read_head(
        &self,
        record: &FileRecord,
        reading: ThumbnailSlot,
        langs: &Languages,
    ) -> Result<HeadRead, Failure> {
        if let Origin::Share { reference } | Origin::Upload { reference, .. } = &record.origin {
            let shares = self.shares.clone();
            let reference = reference.clone();
            let langs = langs.clone();
            let bytes = tokio::task::spawn_blocking(move || {
                let opened = shares
                    .open(&reference)
                    .map_err(|e| share_failure(e, &langs))?;
                let total = opened.snapshot.size;
                let mut file = opened.file.take(MAX_TEXT_PREVIEW_BYTES as u64 + 1);
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .map_err(|e| Failure::internal(format!("读文件失败：{e}")))?;
                Ok::<_, Failure>((bytes, total))
            })
            .await
            .map_err(|e| Failure::internal(format!("读文件的任务中止：{e}")))??;
            let (bytes, total) = bytes;
            return Ok(HeadRead {
                truncated: bytes.len() > MAX_TEXT_PREVIEW_BYTES,
                bytes,
                total: Some(total),
                slot: reading,
            });
        }
        let content = self.content(record, RemoteRequest::whole(), langs).await?;
        match content {
            Content::Remote(response) => {
                // 有 Content-Length 或声明大小时如实报总大小；都没有就如实说未知。
                let total = response
                    .headers()
                    .get(hyper::header::CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok())
                    .or(record.declared_size);
                let bytes =
                    resource::read_upto(response, MAX_TEXT_PREVIEW_BYTES + 1, langs).await?;
                Ok(HeadRead {
                    truncated: bytes.len() > MAX_TEXT_PREVIEW_BYTES,
                    bytes,
                    total,
                    slot: reading,
                })
            }
            Content::Bytes(bytes) => {
                // 这条路（MCP resources/read）本来就整读进来了：截断标记按全长算，这里只留下头一段。
                let truncated = bytes.len() > MAX_TEXT_PREVIEW_BYTES;
                let mut bytes = bytes;
                bytes.truncate(MAX_TEXT_PREVIEW_BYTES + 1);
                Ok(HeadRead {
                    truncated,
                    bytes,
                    total: record.declared_size,
                    slot: reading,
                })
            }
            Content::File(_) | Content::Unread { .. } => {
                Err(Failure::internal("插件的资源读出来不该是文件句柄或未读"))
            }
        }
    }

    /// 缩小的图片，名额和期限在 [`Self::thumbnail`] 和 [`Self::preview`] 之间共用。
    pub(super) async fn scaled(
        &self,
        record: &FileRecord,
        scale: Scale,
        langs: &Languages,
    ) -> Result<Vec<u8>, Failure> {
        let not_viewable = |why: String| Failure::new(FileFailure::NotViewable, why);
        if !can_thumbnail(&record.name, record.mime_type.as_deref()) {
            return Err(not_viewable(l10n::text(langs, "files-no-preview", &[])));
        }
        let reading = self.thumbnail_slot(&record.origin, langs).await?;
        let (data, reading) = tokio::time::timeout(
            self.thumbnail_read_timeout,
            self.read_whole(record, reading, langs, too_big_for_thumbnail),
        )
        .await
        .map_err(|_| {
            thumbnail_read_timed_out(&record.origin, self.thumbnail_read_timeout, langs)
        })??;
        drop(reading);
        let decode_busy = || {
            Failure::new(
                FileFailure::ServiceUnavailable,
                l10n::text(langs, "files-decode-busy", &[]),
            )
        };
        let _decoding = self
            .thumbnail_decodes
            .clone()
            .try_acquire_owned()
            .map_err(|_| decode_busy())?;
        let decoded = async {
            match scale {
                Scale::Thumbnail => self.images.thumbnail(&data).await,
                Scale::Preview => self.images.preview(&data).await,
            }
        };
        tokio::time::timeout(self.thumbnail_decode_timeout, decoded)
            .await
            .map_err(|_| decode_busy())?
            .map_err(|e| match e {
                ImageError::Refused(why) | ImageError::TimedOut(why) => not_viewable(l10n::text(
                    langs,
                    "files-preview-failed",
                    &[("why", why.into())],
                )),
                ImageError::Unavailable(why) => Failure::internal(why),
            })
    }

    /// 读取名额。读卡住时后来的请求不能一直等。本地的等不到不归给来源，这一张可能什么问题都没有；插件的见 [`PluginSlots::acquire`]。
    pub(super) async fn thumbnail_slot(
        &self,
        origin: &Origin,
        langs: &Languages,
    ) -> Result<ThumbnailSlot, Failure> {
        let wait = self.thumbnail_slot_wait;
        match origin {
            Origin::Share { .. } | Origin::Upload { .. } => {
                let pool = self.local_thumbnail_reads.clone();
                tokio::time::timeout(wait, pool.acquire_owned())
                    .await
                    .map(|permit| ThumbnailSlot::Local(permit.expect("信号量不会关闭")))
                    .map_err(|_| {
                        Failure::new(
                            FileFailure::ServiceUnavailable,
                            l10n::text(langs, "files-read-busy", &[]),
                        )
                    })
            }
            Origin::Plugin { plugin, .. } => self
                .plugin_thumbnail_reads
                .acquire(plugin, wait, langs)
                .await
                .map(ThumbnailSlot::Plugin),
        }
    }

    /// 原文件的字节和它占着的名额。超过上限按 `too_big` 报（缩略图和抽文本、读视频的类别不一样）。
    pub(super) async fn read_whole(
        &self,
        record: &FileRecord,
        reading: ThumbnailSlot,
        langs: &Languages,
        too_big: fn(&Languages) -> Failure,
    ) -> Result<(Vec<u8>, ThumbnailSlot), Failure> {
        if let Origin::Share { reference } | Origin::Upload { reference, .. } = &record.origin {
            return self
                .read_local_whole(reference.clone(), reading, langs, too_big)
                .await;
        }
        let content = self
            .content(record, RemoteRequest::whole(), langs)
            .await
            .map_err(|e| match e.kind {
                FileFailure::ResourceTooLarge => too_big(langs),
                _ => e,
            })?;
        let bytes = match content {
            Content::Remote(response) => {
                resource::read_limited(response, MAX_THUMBNAIL_INPUT, langs)
                    .await?
                    .ok_or_else(|| too_big(langs))?
            }
            Content::Bytes(bytes) if bytes.len() > MAX_THUMBNAIL_INPUT => {
                return Err(too_big(langs));
            }
            Content::Bytes(bytes) => bytes,
            Content::File(_) | Content::Unread { .. } => {
                return Err(Failure::internal("插件的资源读出来不该是这样"));
            }
        };
        Ok((bytes, reading))
    }

    /// 阻塞的读放不下来：调用方超时之后它还在跑（存储卡住时可能一直不回来），所以名额跟着它走，真读完才还。打开也在里面，一样会卡。
    async fn read_local_whole(
        &self,
        reference: ShareRef,
        reading: ThumbnailSlot,
        langs: &Languages,
        too_big: fn(&Languages) -> Failure,
    ) -> Result<(Vec<u8>, ThumbnailSlot), Failure> {
        let shares = self.shares.clone();
        let langs = langs.clone();
        #[cfg(test)]
        let stalled = self.local_reads_stalled.clone();
        tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            let _stalled = stalled.blocking_read();
            let opened = shares
                .open(&reference)
                .map_err(|e| share_failure(e, &langs))?;
            if opened.snapshot.size > MAX_THUMBNAIL_INPUT as u64 {
                return Err(too_big(&langs));
            }
            // 文件可能在查过大小之后变大。
            let mut file = opened.file.take(MAX_THUMBNAIL_INPUT as u64 + 1);
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|e| Failure::internal(format!("读文件失败：{e}")))?;
            if bytes.len() > MAX_THUMBNAIL_INPUT {
                return Err(too_big(&langs));
            }
            Ok((bytes, reading))
        })
        .await
        .map_err(|e| Failure::internal(format!("读文件的任务中止：{e}")))?
    }
}

/// 缩到多大。
#[derive(Clone, Copy)]
pub(super) enum Scale {
    Thumbnail,
    Preview,
}

/// 占着的读取名额，读完才放下。
pub(super) enum ThumbnailSlot {
    Local(#[allow(dead_code)] OwnedSemaphorePermit),
    Plugin(#[allow(dead_code)] PluginSlot),
}

pub(super) fn too_big_for_thumbnail(langs: &Languages) -> Failure {
    Failure::new(
        FileFailure::NotViewable,
        l10n::text(
            langs,
            "files-too-big-for-thumbnail",
            &[(
                "limit",
                wonton_l10n::Arg::from((MAX_THUMBNAIL_INPUT >> 20) as i64),
            )],
        ),
    )
}

/// `view_file` 抽 PDF 文字、读视频元数据的上限：和缩略图一样 64 MiB，但类别是 `resource_too_large`（ADR-0104）。
pub(super) fn too_big_for_parsing(langs: &Languages) -> Failure {
    Failure::new(
        FileFailure::ResourceTooLarge,
        l10n::text(
            langs,
            "files-too-big-for-parsing",
            &[(
                "limit",
                wonton_l10n::Arg::from((MAX_THUMBNAIL_INPUT >> 20) as i64),
            )],
        ),
    )
}

/// 这个来源的文件多大还做得了缩略图：没有内网地址的插件资源只能经 `resources/read` 读。
pub(super) fn thumbnail_input_limit(origin: &Origin) -> u64 {
    let limit = match origin {
        Origin::Plugin {
            internal_url: None, ..
        } => MAX_THUMBNAIL_INPUT.min(MAX_RESOURCE_BYTES),
        _ => MAX_THUMBNAIL_INPUT,
    };
    limit as u64
}

/// 插件读不完和 `/content` 一样算插件不可用；本地文件读不完不是插件的事，是核心这边（比如磁盘）出了问题。
pub(super) fn thumbnail_read_timed_out(
    origin: &Origin,
    after: Duration,
    langs: &Languages,
) -> Failure {
    let why = l10n::text(
        langs,
        "files-thumbnail-timeout",
        &[("after", format!("{after:?}").into())],
    );
    match origin {
        Origin::Plugin { .. } => Failure::new(FileFailure::PluginUnavailable, why),
        Origin::Share { .. } | Origin::Upload { .. } => Failure::internal(why),
    }
}
