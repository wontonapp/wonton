//! 交给 App 的插件图标：地址按原文件的 sha256 和规格生成（`/_wonton/api/images/<sha256>/icon-192.png`），同一个地址的
//! 内容不变，App 可以一直缓存。核心只给它列出的插件的图标：已安装的（图标在它们的记录里）、最近预览过的插件包，
//! 和商店最近一次列出的插件。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tokio::sync::OnceCell;
use wonton_protocol::{PluginIcon, PluginImage};

use super::image_worker::{ImageError, ImageWorker};
use super::images::{ICON_LARGE, ICON_SMALL, Icon, IconFormat};
use super::package::PluginPackage;
use super::package_files::sha256_hex;

/// 图片地址的前缀。
pub const IMAGES_PATH: &str = "/_wonton/api/images";

/// 记住最近这么多个预览过的插件包的图标，供确认页显示。
const MAX_PREVIEWS: usize = 32;
/// 处理结果（包括处理不了的原因）最多缓存这么多个，满了丢掉最早的。
const MAX_RENDERED: usize = 128;

/// 插件包里的图标原文件。
#[derive(Debug, Clone)]
pub struct IconSource {
    pub path: String,
    pub sha256: String,
    pub format: IconFormat,
    pub data: Arc<Vec<u8>>,
}

impl IconSource {
    /// 插件包声明的图标；没有图标时为空。
    pub fn of(package: &PluginPackage) -> Option<Self> {
        package
            .manifest
            .icon
            .as_deref()
            .and_then(|path| Self::of_path(package, path))
    }

    /// 插件包里指定的图标文件。
    pub fn of_path(package: &PluginPackage, raw: &str) -> Option<Self> {
        let path = super::package_files::normalize(raw).ok()?;
        let data = package.files().get(&path)?;
        Some(Self {
            sha256: sha256_hex(data),
            format: IconFormat::from_path(&path)?,
            data: Arc::new(data.clone()),
            path,
        })
    }

    /// 一个图标文件；不是 PNG 或 SVG 时为空。
    pub fn from_file(path: &str, data: Vec<u8>) -> Option<Self> {
        Some(Self {
            path: path.to_owned(),
            sha256: sha256_hex(&data),
            format: IconFormat::from_path(path)?,
            data: Arc::new(data),
        })
    }

    /// 供底栏使用的 192×192 PNG 地址，和普通插件图标分开命名。
    pub fn tab_image(&self) -> PluginImage {
        PluginImage {
            path: format!("{IMAGES_PATH}/{}/tab-icon-{ICON_SMALL}.png", self.sha256),
        }
    }

    pub fn paths(&self) -> PluginIcon {
        let image = |edge: u32| PluginImage {
            path: format!("{IMAGES_PATH}/{}/icon-{edge}.png", self.sha256),
        };
        PluginIcon {
            small: image(ICON_SMALL),
            large: image(ICON_LARGE),
        }
    }
}

/// 图片地址里的文件名对应哪一张。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Small,
    Large,
    TabSmall,
    TabLarge,
}

impl Variant {
    pub fn parse(file: &str) -> Option<Self> {
        let (prefix, tab) = if let Some(file) = file.strip_prefix("tab-icon-") {
            (file, true)
        } else {
            (file.strip_prefix("icon-")?, false)
        };
        match prefix.strip_suffix(".png")?.parse() {
            Ok(ICON_SMALL) if tab => Some(Self::TabSmall),
            Ok(ICON_LARGE) if tab => Some(Self::TabLarge),
            Ok(ICON_SMALL) => Some(Self::Small),
            Ok(ICON_LARGE) => Some(Self::Large),
            _ => None,
        }
    }
}

pub struct Gallery {
    images: Arc<ImageWorker>,
    previews: Mutex<VecDeque<IconSource>>,
    /// 商店最近一次列出的插件的图标。
    store: Mutex<HashMap<String, IconSource>>,
    rendered: Mutex<Rendered>,
}

/// 一张图标的处理结果（处理不了时是原因）：并发的请求等同一次处理。
type Rendering = Arc<OnceCell<Result<Arc<Icon>, String>>>;

#[derive(Default)]
struct Rendered {
    icons: HashMap<String, Rendering>,
    order: VecDeque<String>,
}

impl Gallery {
    pub fn new(images: impl Into<Arc<ImageWorker>>) -> Self {
        Self {
            images: images.into(),
            previews: Mutex::default(),
            store: Mutex::default(),
            rendered: Mutex::default(),
        }
    }

    /// 校验插件包的图标和截图用同一个进程（`plugin check` 的同一份代码）。
    pub(super) fn images(&self) -> &Arc<ImageWorker> {
        &self.images
    }

    /// 记住预览过的插件包的图标：确认页显示它的时候，插件还没装。
    pub fn remember_preview(&self, source: &IconSource) {
        let mut previews = self.previews.lock().expect("图标表锁中毒");
        previews.retain(|p| p.sha256 != source.sha256);
        previews.push_back(source.clone());
        if previews.len() > MAX_PREVIEWS {
            previews.pop_front();
        }
    }

    pub fn preview(&self, sha256: &str) -> Option<IconSource> {
        let previews = self.previews.lock().expect("图标表锁中毒");
        let found = previews.iter().find(|p| p.sha256 == sha256).cloned();
        drop(previews);
        found.or_else(|| {
            self.store
                .lock()
                .expect("图标表锁中毒")
                .get(sha256)
                .cloned()
        })
    }

    /// 记住商店列出的插件的图标，换掉上一次列出的。
    pub fn remember_store(&self, icons: impl IntoIterator<Item = IconSource>) {
        let icons = icons.into_iter().map(|i| (i.sha256.clone(), i)).collect();
        *self.store.lock().expect("图标表锁中毒") = icons;
    }

    /// 处理图标（在子进程里，限时限内存），结果缓存起来。同一张图标只处理一次：处理不了的（包括超时、内存不够）也记住，
    /// 不会被反复拿来占用子进程；起不了子进程不是图标的问题，不记，下次再试。
    pub async fn render(&self, source: &IconSource) -> Result<Arc<Icon>, ImageError> {
        let rendering = self.rendering(&source.sha256);
        let result = rendering
            .get_or_try_init(|| async {
                match self.images.icon(source.format, &source.data).await {
                    Ok(icon) => Ok(Ok(Arc::new(icon))),
                    Err(ImageError::Refused(reason)) => Ok(Err(reason)),
                    Err(e @ ImageError::Unavailable(_)) => Err(e),
                }
            })
            .await?;
        result.clone().map_err(ImageError::Refused)
    }

    /// 校验并渲染底栏图标。和普通图标分开缓存，避免同一文件的两套规则互相复用。
    pub async fn render_tab(&self, source: &IconSource) -> Result<Arc<Icon>, ImageError> {
        let key = format!("tab:{}", source.sha256);
        let rendering = self.rendering(&key);
        let result = rendering
            .get_or_try_init(|| async {
                match self.images.tab_icon(source.format, &source.data).await {
                    Ok(icon) => Ok(Ok(Arc::new(icon))),
                    Err(ImageError::Refused(reason)) => Ok(Err(reason)),
                    Err(e @ ImageError::Unavailable(_)) => Err(e),
                }
            })
            .await?;
        result.clone().map_err(ImageError::Refused)
    }

    fn rendering(&self, sha256: &str) -> Rendering {
        let mut rendered = self.rendered.lock().expect("图标缓存锁中毒");
        if let Some(rendering) = rendered.icons.get(sha256) {
            return rendering.clone();
        }
        if rendered.icons.len() >= MAX_RENDERED
            && let Some(oldest) = rendered.order.pop_front()
        {
            rendered.icons.remove(&oldest);
        }
        let rendering = Rendering::default();
        rendered.icons.insert(sha256.to_owned(), rendering.clone());
        rendered.order.push_back(sha256.to_owned());
        rendering
    }
}

#[cfg(test)]
mod tests;
