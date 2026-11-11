//! 链接预览的客户端一侧（ADR-0065）：发送框和时间线共用的按网址缓存，事件里 `com.beeper.linkpreviews`
//! 的读和写（MSC4095 的 unstable 字段，字段名跟着上游改）。
//!
//! 同一网址一次会话只向家里的核心取一次，成功和失败都记着：时间线每次刷新都重取会把请求打爆，核心那头
//! 还按设备限速。结果只在本机内存里，换服务器、重启核心就清掉。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{oneshot, watch};
use wonton_matrix::{
    ChatRoom, Error, Message, MessageBody, OwnedRoomId, PreviewImageSource, encrypted_media_token,
    media_token,
};
use wonton_protocol::{
    CoreError, LinkPreview, LinkPreviewCard, LinkPreviewCardImage, LinkPreviewFailure,
    LinkPreviewReason, LinkPreviewState, MediaHandle,
};

use super::draft::DraftWrite;
use super::links::{ExternalLink, LinkTargets, first_external_link};
use super::{ChatLink, lock};
use crate::plugins::api::{CoreApi, CoreApiError, to_core_error};

/// 事件内容里带预览的字段（MSC4095 未定稿前的名字）。
pub(super) const BUNDLED_FIELD: &str = "com.beeper.linkpreviews";

/// 一次预览请求等核心多久：核心自己整次限 10 秒，加上网络和图片的余量。
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(30);

/// 链接预览的缓存：一个网址一份结果，取的时候单飞，落定了通知订阅者。
#[derive(Clone)]
pub(crate) struct Previews {
    inner: Arc<Inner>,
}

struct Inner {
    /// 网址 → 结果（卡片或按类别的失败）。
    outcomes: Mutex<HashMap<String, LinkPreviewState>>,
    /// 正在取的网址 → 等结果的接收端：并发的调用共用同一次请求。
    in_flight: Mutex<HashMap<String, watch::Receiver<Option<LinkPreviewState>>>>,
    /// 自上次 [`Previews::take_settled`] 后落定的网址：时间线只重发这些网址所在的条目。
    settled: Mutex<Vec<String>>,
    /// 有结果落定时递增：时间线的 select 靠它醒过来。
    version: watch::Sender<u64>,
    /// 主人点掉了预览的房间和当时的第一个链接：草稿的第一个链接还是它就不再出。
    dismissed: Mutex<HashMap<OwnedRoomId, String>>,
}

impl Previews {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                outcomes: Mutex::default(),
                in_flight: Mutex::default(),
                settled: Mutex::default(),
                version: watch::Sender::new(0),
                dismissed: Mutex::default(),
            }),
        }
    }

    /// 已经知道的结果；还没取到时是 `None`。
    pub(crate) fn get(&self, url: &str) -> Option<LinkPreviewState> {
        lock(&self.inner.outcomes).get(url).cloned()
    }

    /// 确保这个网址在取（已在取就不重复），不等结果。时间线的批处理用它。
    pub(crate) fn ensure(&self, url: String, api: &CoreApi) {
        self.spawn(url, api);
    }

    /// 确保在取并等结果：发送框第一次读草稿就要带上预览。
    pub(crate) async fn resolve(&self, url: String, api: &CoreApi) -> LinkPreviewState {
        match self.spawn(url.clone(), api) {
            Some(mut received) => received
                .wait_for(|state| state.is_some())
                .await
                .expect("取预览的任务活着，落定时一定有结果")
                .clone()
                .expect("落定时一定有结果"),
            None => self.get(&url).expect("已在缓存里"),
        }
    }

    /// 已在缓存或已在取时返回 `None`，返回接收端表示这次调用排进了已在取的那一场。
    fn spawn(
        &self,
        url: String,
        api: &CoreApi,
    ) -> Option<watch::Receiver<Option<LinkPreviewState>>> {
        if self.get(&url).is_some() {
            return None;
        }
        let mut in_flight = lock(&self.inner.in_flight);
        if let Some(received) = in_flight.get(&url) {
            return Some(received.clone());
        }
        let (sender, received) = watch::channel(None);
        in_flight.insert(url.clone(), received.clone());
        let inner = self.inner.clone();
        let api = api.clone();
        tokio::spawn(async move {
            let state = fetch(&api, &url).await;
            inner.settle(url, sender, state);
        });
        Some(received)
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.inner.version.subscribe()
    }

    /// 落定了结果的网址，取走就没了。
    pub(crate) fn take_settled(&self) -> Vec<String> {
        std::mem::take(&mut lock(&self.inner.settled))
    }

    /// 主人点掉了这个房间的预览：记下草稿当时的第一个链接。
    pub(crate) fn dismiss(&self, room_id: &OwnedRoomId, url: String) {
        lock(&self.inner.dismissed).insert(room_id.clone(), url);
    }

    /// 草稿的第一个外部链接还是主人点掉的那个。
    pub(crate) fn dismissed(&self, room_id: &OwnedRoomId, url: &str) -> bool {
        lock(&self.inner.dismissed)
            .get(room_id)
            .is_some_and(|recorded| recorded == url)
    }

    /// 第一个链接换了或没有了：点掉的记录作废，草稿换了链接要跟着换预览。
    pub(crate) fn retain_dismissal(&self, room_id: &OwnedRoomId, first: Option<&str>) {
        let mut dismissed = lock(&self.inner.dismissed);
        match (dismissed.get(room_id), first) {
            (Some(recorded), Some(first)) if recorded == first => {}
            _ => {
                dismissed.remove(room_id);
            }
        }
    }

    /// 草稿发出去或清掉了：点掉的记录跟着作废，下次打同样的链接重新出预览。
    pub(crate) fn forget(&self, room_id: &OwnedRoomId) {
        lock(&self.inner.dismissed).remove(room_id);
    }
}

impl Inner {
    fn settle(
        &self,
        url: String,
        sender: watch::Sender<Option<LinkPreviewState>>,
        state: LinkPreviewState,
    ) {
        lock(&self.outcomes).insert(url.clone(), state.clone());
        lock(&self.settled).push(url.clone());
        self.version.send_modify(|version| *version += 1);
        sender.send_replace(Some(state));
        lock(&self.in_flight).remove(&url);
    }
}

async fn fetch(api: &CoreApi, url: &str) -> LinkPreviewState {
    match api
        .get_detailed::<LinkPreview>(&["link-preview"], &[("url", url)], PREVIEW_TIMEOUT)
        .await
    {
        Ok(preview) => LinkPreviewState::Ready {
            card: core_card(preview),
        },
        Err(error) => LinkPreviewState::Failed {
            reason: reason(&error),
        },
    }
}

/// 核心回的预览：图片在核心的缓存里，凭 id 取字节（[`ChatLink::preview_image`]）。
fn core_card(preview: LinkPreview) -> LinkPreviewCard {
    LinkPreviewCard {
        url: preview.url,
        title: preview.title,
        description: preview.description,
        site_name: preview.site_name,
        image: preview
            .image
            .map(|image| LinkPreviewCardImage::Core { image }),
    }
}

/// 核心的错误类别：响应里的 `kind` 原样映射，认不出的按 [`LinkPreviewFailure::Other`]，核心够不着的是
/// `Transport`。
fn reason(error: &CoreApiError) -> LinkPreviewReason {
    match error {
        CoreApiError::Transport(_) => LinkPreviewReason::Transport,
        CoreApiError::Http { kind, .. } => LinkPreviewReason::Upstream {
            reason: kind
                .as_deref()
                .map(failure_of)
                .unwrap_or(LinkPreviewFailure::Other),
        },
        CoreApiError::Protocol(_) => LinkPreviewReason::Upstream {
            reason: LinkPreviewFailure::Other,
        },
    }
}

fn failure_of(kind: &str) -> LinkPreviewFailure {
    serde_json::from_value(Value::String(kind.to_owned())).unwrap_or(LinkPreviewFailure::Other)
}

/// 事件里带的预览（只认第一个，MSC4095）：`matched_url` 必须出现在正文里；条目里只有 `matched_url`
/// 没有别的字段时不算带预览（发送方只给了白名单），照常向核心取。
pub(crate) fn bundled(text: &str, custom: Option<&Value>) -> Option<LinkPreviewState> {
    for entry in custom?.as_array()? {
        let Some(matched) = text_field(entry, "matched_url") else {
            continue;
        };
        if !text.contains(&matched) || entry.as_object().is_some_and(|object| object.len() <= 1) {
            continue;
        }
        return Some(LinkPreviewState::Ready {
            card: LinkPreviewCard {
                url: text_field(entry, "og:url").unwrap_or(matched),
                title: text_field(entry, "og:title"),
                description: text_field(entry, "og:description"),
                site_name: text_field(entry, "og:site_name"),
                image: bundled_image(entry),
            },
        });
    }
    None
}

fn text_field(entry: &Value, name: &str) -> Option<String> {
    entry.get(name).and_then(Value::as_str).map(str::to_owned)
}

fn size_field(entry: &Value, name: &str) -> Option<u32> {
    entry
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|size| u32::try_from(size).ok())
}

/// 图片：加密房间是 `beeper:image:encryption`（EncryptedFile 的 JSON，原样当句柄），未加密是 `og:image`
/// 的 mxc 地址。写坏了的图片不挡卡片，只是没图。
fn bundled_image(entry: &Value) -> Option<LinkPreviewCardImage> {
    let image = if let Some(file) = entry.get("beeper:image:encryption") {
        // 事件里是裸的 EncryptedFile；`encrypted_media_token` 包上媒体句柄认的那一层 `"file"`，
        // 不包会被当成未加密的 mxc，取回来就是密文。
        let token = encrypted_media_token(file)
            .inspect_err(|error| tracing::warn!("预览图片的加密信息读不了，卡片不带图：{error}"))
            .ok()?;
        (MediaHandle { token }, text_field(entry, "og:image:type"))
    } else {
        let mxc = text_field(entry, "og:image")?;
        let token = media_token(&mxc)
            .inspect_err(|error| tracing::warn!("预览图片的地址读不了，卡片不带图：{error}"))
            .ok()?;
        (MediaHandle { token }, text_field(entry, "og:image:type"))
    };
    Some(LinkPreviewCardImage::Matrix {
        media: image.0,
        mime_type: image.1,
        width: size_field(entry, "og:image:width"),
        height: size_field(entry, "og:image:height"),
    })
}

/// 发送时写进事件内容的外壳字段：第一个预览，图片已传到媒体库。每条消息只带第一个（ADR-0065）。
pub(crate) fn event_extra(
    matched: &str,
    card: &LinkPreviewCard,
    image: Option<(PreviewImageSource, wonton_protocol::LinkPreviewImage)>,
) -> serde_json::Map<String, Value> {
    let mut entry = serde_json::Map::new();
    entry.insert("matched_url".to_owned(), json!(matched));
    entry.insert("og:url".to_owned(), json!(card.url));
    if let Some(title) = &card.title {
        entry.insert("og:title".to_owned(), json!(title));
    }
    if let Some(description) = &card.description {
        entry.insert("og:description".to_owned(), json!(description));
    }
    if let Some(site_name) = &card.site_name {
        entry.insert("og:site_name".to_owned(), json!(site_name));
    }
    if let Some((source, meta)) = image {
        entry.insert("og:image:type".to_owned(), json!(meta.mime_type));
        if let Some(width) = meta.width {
            entry.insert("og:image:width".to_owned(), json!(width));
        }
        if let Some(height) = meta.height {
            entry.insert("og:image:height".to_owned(), json!(height));
        }
        match source {
            PreviewImageSource::Plain { mxc } => {
                entry.insert("og:image".to_owned(), json!(mxc));
            }
            PreviewImageSource::Encrypted { file } => {
                entry.insert("beeper:image:encryption".to_owned(), file);
            }
        }
    }
    let mut extra = serde_json::Map::new();
    extra.insert(
        BUNDLED_FIELD.to_owned(),
        Value::Array(vec![Value::Object(entry)]),
    );
    extra
}

/// 这条消息要出预览的网址：事件里带了预览的不取，正文里第一个外部链接的才取。
pub(crate) fn message_target(message: &Message, links: &LinkTargets) -> Option<ExternalLink> {
    let text = match &message.body {
        MessageBody::Text(text) | MessageBody::Notice(text) => text,
        _ => return None,
    };
    if bundled(text, message.custom.get(BUNDLED_FIELD)).is_some() {
        return None;
    }
    first_external_link(text, links)
}

/// 消息的预览：事件里带的直接用；没有的用核心取回的缓存结果，还在取的等下一批再带上。
pub(crate) fn message_preview(
    message: &Message,
    links: &LinkTargets,
    previews: &Previews,
) -> Option<LinkPreviewState> {
    let text = match &message.body {
        MessageBody::Text(text) | MessageBody::Notice(text) => text,
        _ => return None,
    };
    if let Some(preview) = bundled(text, message.custom.get(BUNDLED_FIELD)) {
        return Some(preview);
    }
    let found = first_external_link(text, links)?;
    previews.get(&found.url)
}

impl ChatLink {
    /// 草稿的预览：第一个外部链接、没被点掉的，向核心取（缓存里有的直接用）。核心够不着时按类别给
    /// `Transport`：发送框照常显示取不到的原因，不出假卡片。
    pub(super) async fn draft_preview(&self, url: String) -> LinkPreviewState {
        match self.plugins.api() {
            Ok(api) => self.previews.resolve(url, &api).await,
            Err(_) => LinkPreviewState::Failed {
                reason: LinkPreviewReason::Transport,
            },
        }
    }

    /// 发送时随事件带的外壳字段。没有链接、被点掉或取不到预览时是 `Ok(None)`，不带（取不到的类别发送框
    /// 里已经显示过，主人还是要发）；图片取回来或传不上去时是 `Err`，正文照发、原因报给主人。
    pub(super) async fn preview_extra(
        self: &Arc<Self>,
        room: &ChatRoom,
        room_id: &OwnedRoomId,
        body: &str,
    ) -> Result<Option<serde_json::Map<String, Value>>, Error> {
        let links = LinkTargets::of(&self.plugins.view());
        let Some(found) = first_external_link(body, &links) else {
            return Ok(None);
        };
        if self.previews.dismissed(room_id, &found.url) {
            return Ok(None);
        }
        let api = self
            .plugins
            .api()
            .map_err(|problem| Error::Upstream(problem.message))?;
        let LinkPreviewState::Ready { mut card } = self.previews.resolve(found.url, &api).await
        else {
            return Ok(None);
        };
        let image = match card.image.take() {
            None => None,
            Some(LinkPreviewCardImage::Core { image }) => {
                let bytes = api
                    .get_bytes_detailed(&["link-preview", "images", &image.id], PREVIEW_TIMEOUT)
                    .await
                    .map_err(attachment_error)?;
                let source = room
                    .upload_preview_image(&image.mime_type, bytes)
                    .await
                    .map_err(|error| Error::Upstream(format!("预览图片传不上去：{error}")))?;
                Some((source, image))
            }
            // 事件里带来的 Matrix 来源只出现在收到的消息上，核心给的预览里不会有。
            Some(LinkPreviewCardImage::Matrix { .. }) => None,
        };
        Ok(Some(event_extra(&found.raw, &card, image)))
    }

    /// 预览图片的原文件：核心按 id 把缓存的图片给回来。
    pub(crate) async fn preview_image(&self, id: &str) -> Result<Vec<u8>, CoreError> {
        let api = self.plugins.api().map_err(to_core_error)?;
        api.get_bytes_detailed(&["link-preview", "images", id], PREVIEW_TIMEOUT)
            .await
            .map_err(|error| match error {
                CoreApiError::Transport(problem) => to_core_error(problem),
                CoreApiError::Http { message, .. } | CoreApiError::Protocol(message) => {
                    CoreError::Upstream { reason: message }
                }
            })
    }

    /// 主人点掉了发送框的预览：存着的草稿里第一个链接还是它，这条草稿就不再出。
    pub(super) fn dismiss_preview(self: &Arc<Self>, room_id: &str) {
        let chat = self.clone();
        let room = room_id.to_owned();
        self.runtime.spawn(async move {
            let Ok((_, room_id)) = chat.target(&room) else {
                return;
            };
            let (ack, received) = oneshot::channel();
            chat.enqueue_draft(DraftWrite::Read(room_id.clone(), ack));
            let Ok(Some(stored)) = received.await else {
                return;
            };
            let links = LinkTargets::of(&chat.plugins.view());
            if let Some(found) = first_external_link(&stored.body, &links) {
                chat.previews.dismiss(&room_id, found.url);
            }
        });
    }
}

/// 预览的接口失败：给发送框的横幅用，不挡正文。
fn attachment_error(error: CoreApiError) -> Error {
    let reason = match error {
        CoreApiError::Transport(_) => "连不上家里的核心".to_owned(),
        CoreApiError::Http { message, .. } | CoreApiError::Protocol(message) => message,
    };
    Error::Upstream(reason)
}
