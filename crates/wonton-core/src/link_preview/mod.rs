//! 链接预览（ADR-0065 的“核心取预览”）：设备给一个网址，核心去取它的标题、摘要和图片。
//!
//! 只有网关上主人和家人的设备能调（`gateway::link_preview`）。出网走主人配的出网代理，没配就直连；不许去内网和本机地址
//! （[`target`]），用检查过的 IP 去连（[`connect`]）。结果按网址缓存（[`cache`]），没命中缓存的才按设备限速（[`limit`]）：
//! 打开一个链接很多的聊天时，读缓存不该被拦。

mod cache;
pub(crate) mod connect;
mod fetch;
mod html;
mod limit;
mod target;

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hyper::body::Bytes;
use hyper::header::{self, HeaderMap};
use url::Url;
use wonton_protocol::{LinkPreview, LinkPreviewFailure, LinkPreviewImage};

pub(crate) use crate::gateway::send_limits::Clock;
use crate::gateway::send_limits::system_clock;
use crate::plugin::egress::core_proxy::CoreProxy;
pub(crate) use cache::Image;
pub(crate) use connect::Net;
use fetch::Overflow;

const ACCEPT_HTML: &str = "text/html,application/xhtml+xml;q=0.9,*/*;q=0.1";
const ACCEPT_IMAGE: &str = "image/png,image/jpeg,image/gif,image/webp;q=0.9,*/*;q=0.1";

/// 取不到的原因：类别给 App 选文字，`detail` 是原始值（状态码、地址、底层错误），进“详情”。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub kind: LinkPreviewFailure,
    pub detail: String,
}

impl Failure {
    pub(crate) fn new(kind: LinkPreviewFailure, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

/// 各项上限，数值见 ADR-0065。测试换成小的。
#[derive(Debug, Clone)]
pub(crate) struct Limits {
    pub per_minute: usize,
    pub timeout: Duration,
    pub max_html: usize,
    pub max_image: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_minute: 30,
            timeout: Duration::from_secs(10),
            max_html: 512 << 10,
            max_image: 5 << 20,
        }
    }
}

pub(crate) struct LinkPreviews {
    net: Net,
    limits: Limits,
    cache: Arc<cache::Cache>,
    rate: limit::RateLimit,
}

impl LinkPreviews {
    /// `dir` 是缓存目录（核心数据目录下）；`proxy` 是核心自己出网用的代理，和推送、商店是同一份。
    pub(crate) fn new(dir: PathBuf, proxy: CoreProxy) -> Self {
        Self::with(dir, Net::system(proxy), Limits::default(), system_clock())
    }

    pub(crate) fn with(dir: PathBuf, net: Net, limits: Limits, now: Clock) -> Self {
        Self {
            rate: limit::RateLimit::new(limits.per_minute, now.clone()),
            cache: Arc::new(cache::Cache::new(dir, now)),
            net,
            limits,
        }
    }

    /// `device` 是限速按的那个键：设备的 endpoint ID，或者主人令牌。
    pub(crate) async fn preview(&self, text: &str, device: &str) -> Result<LinkPreview, Failure> {
        let url = target::parse(text)?;
        let key = cache::key(url.as_str());
        if let Some(cached) = self.cache.get(&key) {
            return cached;
        }
        if !self.rate.take(device) {
            return Err(Failure::new(LinkPreviewFailure::TooManyRequests, device));
        }
        let timeout = self.limits.timeout;
        let fetched = tokio::time::timeout(timeout, self.fetch(url, &key))
            .await
            .unwrap_or_else(|_| {
                let detail = format!("{timeout:?}");
                Err(Failure::new(LinkPreviewFailure::Timeout, detail))
            });
        let (result, image) = match fetched {
            Ok((preview, image)) => (Ok(preview), image),
            Err(failure) => (Err(failure), None),
        };
        let cache = self.cache.clone();
        let stored = result.clone();
        let written = tokio::task::spawn_blocking(move || {
            cache.put(&key, &stored, image.as_deref());
        });
        if let Err(e) = written.await {
            tracing::warn!(error = %e, "链接预览写缓存的任务没有跑完");
        }
        result
    }

    /// 预览图片，过期或没有时是 `None`。
    pub(crate) fn image(&self, id: &str) -> Option<Image> {
        self.cache.image(id)
    }

    async fn fetch(&self, url: Url, key: &str) -> Result<(LinkPreview, Option<Bytes>), Failure> {
        let page = fetch::get(
            &self.net,
            url,
            ACCEPT_HTML,
            self.limits.max_html,
            Overflow::Truncate,
            html_only,
        )
        .await?;
        let head = html::parse(&page.body, charset(&page.headers).as_deref());
        if head.is_empty() {
            return Err(Failure::new(LinkPreviewFailure::NoPreview, page.url));
        }
        let has_text = head.title.is_some() || head.description.is_some();
        let (image, bytes) = match &head.image {
            None => (None, None),
            Some(src) => match self.fetch_image(&page.url, src, key).await {
                Ok((image, bytes)) => (Some(image), Some(bytes)),
                // 有标题或摘要时没有图片也是一张能用的卡片；只有图片的，图片的失败就是预览的失败。
                Err(failure) if has_text => {
                    tracing::info!(page = %page.url, kind = ?failure.kind, detail = %failure.detail, "预览图片取不到，卡片不带图");
                    (None, None)
                }
                Err(failure) => return Err(failure),
            },
        };
        let preview = LinkPreview {
            url: page.url.to_string(),
            title: head.title,
            description: head.description,
            site_name: head.site_name,
            image,
        };
        Ok((preview, bytes))
    }

    /// 取 `og:image`：同样的地址和跳转规矩，只收常见的位图类型，类型按内容认，不信 `Content-Type`。
    async fn fetch_image(
        &self,
        page: &Url,
        src: &str,
        key: &str,
    ) -> Result<(LinkPreviewImage, Bytes), Failure> {
        let url = page
            .join(src)
            .map_err(|e| Failure::new(LinkPreviewFailure::InvalidUrl, e.to_string()))?;
        let url = target::check(url)?;
        let max = self.limits.max_image;
        let got = fetch::get(
            &self.net,
            url,
            ACCEPT_IMAGE,
            max,
            Overflow::Fail,
            declared_fits,
        )
        .await?;
        let format = image::guess_format(&got.body).ok();
        let mime_type = match format {
            Some(image::ImageFormat::Png) => "image/png",
            Some(image::ImageFormat::Jpeg) => "image/jpeg",
            Some(image::ImageFormat::Gif) => "image/gif",
            Some(image::ImageFormat::WebP) => "image/webp",
            other => return Err(fetch::upstream(format!("{other:?}"))),
        };
        let (width, height) = format
            .and_then(|format| {
                image::ImageReader::with_format(Cursor::new(&got.body[..]), format)
                    .into_dimensions()
                    .ok()
            })
            .map_or((None, None), |(w, h)| (Some(w), Some(h)));
        let image = LinkPreviewImage {
            id: key.to_owned(),
            mime_type: mime_type.to_owned(),
            width,
            height,
        };
        Ok((image, got.body))
    }
}

/// 声明的大小超过上限就不读了。
fn declared_fits(headers: &HeaderMap, max: usize) -> Result<(), Failure> {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    match declared {
        Some(size) if size > max as u64 => Err(fetch::upstream(format!("{size} 字节"))),
        _ => Ok(()),
    }
}

/// 不是网页就不读响应体。没写类型的照样当网页解析，解析不出东西就是 `no_preview`。
fn html_only(headers: &HeaderMap, _: usize) -> Result<(), Failure> {
    let Some(value) = headers.get(header::CONTENT_TYPE) else {
        return Ok(());
    };
    let text = value.to_str().unwrap_or_default();
    let essence = text.split(';').next().unwrap_or_default().trim();
    if essence.eq_ignore_ascii_case("text/html")
        || essence.eq_ignore_ascii_case("application/xhtml+xml")
    {
        return Ok(());
    }
    Err(Failure::new(LinkPreviewFailure::NotHtml, essence))
}

/// `Content-Type` 里的 `charset`。
fn charset(headers: &HeaderMap) -> Option<String> {
    let text = headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
    text.split(';').skip(1).find_map(|param| {
        let (name, value) = param.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"').to_owned())
    })
}

#[cfg(test)]
mod proxy_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
