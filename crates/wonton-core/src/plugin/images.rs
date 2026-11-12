//! 插件的图标和截图（ADR-0017 的“图标和截图”）：解码插件给的图片，按固定规格重新编码后才交给 App，App 收不到
//! SVG，也收不到原文件的元数据。
//!
//! 图片是不可信的输入：文件大小和解码后的像素数都有上限。SVG 只用 resvg 渲染：不取外部资源、不解码嵌入的位图、
//! 不渲染文字，不收 DTD；交给 resvg 之前先检查一遍（[`preflight`]）。这些检查挡不住所有费时费内存的输入，所以核心只在
//! 限了内存和时间的子进程里调用这里的函数（见 [`super::image_worker`]）。

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbImage, RgbaImage};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, ImageHrefResolver, roxmltree};

use super::languages::Languages;

mod preflight;

/// 图标输出的两种边长：列表和主屏用小的，详情页和确认页用大的。
pub const ICON_SMALL: u32 = 192;
pub const ICON_LARGE: u32 = 512;

const MAX_PNG_ICON_BYTES: usize = 512 << 10;
const MAX_SVG_ICON_BYTES: usize = 128 << 10;
const MAX_SCREENSHOT_BYTES: usize = 1 << 20;
/// 几种图片里最大的文件上限（截图的）。
pub const MAX_FILE_BYTES: usize = MAX_SCREENSHOT_BYTES;
/// PNG 图标至少这么大，放大到 512 才不糊。
const MIN_ICON_EDGE: u32 = 256;
/// 解码后的宽和高都不超过它。
const MAX_EDGE: u32 = 4096;
/// 截图的长边缩到不超过它。
const SCREENSHOT_EDGE: u32 = 1280;
const JPEG_QUALITY: u8 = 85;
/// 文件缩略图的长边（ADR-0020，同 ADR-0005）。
const THUMBNAIL_EDGE: u32 = 800;
const PREVIEW_EDGE: u32 = 1024;
/// 做缩略图时原图的大小上限，手机和相机的照片都在里面。
pub const MAX_THUMBNAIL_INPUT: usize = 64 << 20;
/// 做缩略图时原图的宽和高的上限。
const THUMBNAIL_MAX_EDGE: u32 = 16384;
/// 交给 XML 解析器的节点数上限（含文字和注释），在数元素之前挡住超大的文件。
const MAX_XML_NODES: u32 = 100_000;

/// 图标的格式，按清单里的文件名认。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconFormat {
    Png,
    Svg,
}

impl IconFormat {
    pub fn from_path(path: &str) -> Option<Self> {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".png") {
            Some(Self::Png)
        } else if lower.ends_with(".svg") {
            Some(Self::Svg)
        } else {
            None
        }
    }
}

/// 处理好的图标：两种边长的 PNG。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub small: Vec<u8>,
    pub large: Vec<u8>,
    /// 四个角里有透明的：图标应该满幅，圆角由 App 裁，插件自己画了圆角或留了透明边时 `plugin check` 提醒。
    pub transparent_corner: bool,
}

/// 处理好的截图：长边不超过 1280 的 JPEG，和原图的宽高。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// 普通插件图标的处理。
pub fn icon(format: IconFormat, data: &[u8], langs: &Languages) -> Result<Icon, String> {
    encode_icon(icon_pixels(format, data, langs)?, langs)
}

/// 底栏图标只取透明通道表达形状，并且必须同时有透明和非透明像素。
pub fn tab_icon(format: IconFormat, data: &[u8], langs: &Languages) -> Result<Icon, String> {
    let mut pixels = tab_icon_pixels(format, data, langs)?;
    validate_tab_icon(&pixels, langs)?;
    for pixel in pixels.pixels_mut() {
        pixel.0[..3].fill(u8::MAX);
    }
    encode_icon(pixels, langs)
}

fn encode_icon(large: RgbaImage, langs: &Languages) -> Result<Icon, String> {
    let small = imageops::resize(&large, ICON_SMALL, ICON_SMALL, FilterType::Lanczos3);
    Ok(Icon {
        transparent_corner: has_transparent_corner(&large),
        small: encode_png(small, langs)?,
        large: encode_png(large, langs)?,
    })
}

fn validate_tab_icon(pixels: &RgbaImage, langs: &Languages) -> Result<(), String> {
    let all_opaque = pixels.pixels().all(|pixel| pixel[3] == u8::MAX);
    let all_transparent = pixels.pixels().all(|pixel| pixel[3] == 0);
    if all_opaque || all_transparent {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-tab-icon-alpha",
            &[],
        ));
    }
    Ok(())
}

/// 图标画成 512×512 的 RGBA：PNG 解码后缩放，SVG 用 resvg 渲染。
fn icon_pixels(format: IconFormat, data: &[u8], langs: &Languages) -> Result<RgbaImage, String> {
    match format {
        IconFormat::Png => png_icon(data, MIN_ICON_EDGE, langs),
        IconFormat::Svg => svg_icon(data, langs),
    }
}

fn tab_icon_pixels(
    format: IconFormat,
    data: &[u8],
    langs: &Languages,
) -> Result<RgbaImage, String> {
    match format {
        IconFormat::Png => png_icon(data, 1, langs),
        IconFormat::Svg => svg_icon(data, langs),
    }
}

fn has_transparent_corner(pixels: &RgbaImage) -> bool {
    let (w, h) = pixels.dimensions();
    [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)]
        .iter()
        .any(|&(x, y)| pixels.get_pixel(x, y)[3] < 128)
}

pub fn screenshot(data: &[u8], langs: &Languages) -> Result<Screenshot, String> {
    if data.len() > MAX_SCREENSHOT_BYTES {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-too-many-kib",
            &[("kib", (MAX_SCREENSHOT_BYTES >> 10).into())],
        ));
    }
    let format = image::guess_format(data)
        .map_err(|_| crate::l10n::text(langs, "plugin-install-img-not-png-or-jpeg", &[]))?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-is-format",
            &[("format", format!("{format:?}").into())],
        ));
    }
    let image = decode(data, format, langs)?;
    let (width, height) = (image.width(), image.height());
    let image = if width.max(height) > SCREENSHOT_EDGE {
        image.resize(SCREENSHOT_EDGE, SCREENSHOT_EDGE, FilterType::Lanczos3)
    } else {
        image
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode_image(&on_white(&image.to_rgba8()))
        .map_err(|e| {
            crate::l10n::text(
                langs,
                "plugin-install-img-encode-jpeg-failed",
                &[("error", e.to_string().into())],
            )
        })?;
    Ok(Screenshot {
        jpeg,
        width,
        height,
    })
}

/// 文件的缩略图：长边 800 像素的 JPEG。
pub fn thumbnail(data: &[u8], langs: &Languages) -> Result<Vec<u8>, String> {
    scaled_jpeg(data, THUMBNAIL_EDGE, langs)
}

/// 给 Bot 看的预览：长边 1024 像素的 JPEG（ADR-0020 的“Bot”）。
pub fn preview(data: &[u8], langs: &Languages) -> Result<Vec<u8>, String> {
    scaled_jpeg(data, PREVIEW_EDGE, langs)
}

/// 缩到长边 `edge` 的 JPEG，按 EXIF 转正。重新编码后不带原文件的元数据（拍摄地点这些）。
fn scaled_jpeg(data: &[u8], edge: u32, langs: &Languages) -> Result<Vec<u8>, String> {
    if data.len() > MAX_THUMBNAIL_INPUT {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-too-many-mib",
            &[("mib", (MAX_THUMBNAIL_INPUT >> 20).into())],
        ));
    }
    let format = image::guess_format(data)
        .map_err(|_| crate::l10n::text(langs, "plugin-install-img-not-supported-format", &[]))?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-format-no-thumbnail",
            &[("format", format!("{format:?}").into())],
        ));
    }
    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(THUMBNAIL_MAX_EDGE);
    limits.max_image_height = Some(THUMBNAIL_MAX_EDGE);
    limits.max_alloc = Some(384 << 20);
    reader.limits(limits);
    let failed = |e: image::ImageError, langs: &Languages| match e {
        image::ImageError::Limits(_) => crate::l10n::text(
            langs,
            "plugin-install-img-over-edge-or-memory",
            &[("max", THUMBNAIL_MAX_EDGE.into())],
        ),
        other => crate::l10n::text(
            langs,
            "plugin-install-img-decode-failed",
            &[("error", other.to_string().into())],
        ),
    };
    let mut decoder = reader.into_decoder().map_err(|e| failed(e, langs))?;
    let orientation = decoder.orientation().map_err(|e| failed(e, langs))?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| failed(e, langs))?;
    image.apply_orientation(orientation);
    if image.width().max(image.height()) > edge {
        image = image.resize(edge, edge, FilterType::Triangle);
    }
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode_image(&on_white(&image.to_rgba8()))
        .map_err(|e| {
            crate::l10n::text(
                langs,
                "plugin-install-img-encode-jpeg-failed",
                &[("error", e.to_string().into())],
            )
        })?;
    Ok(jpeg)
}

fn png_icon(data: &[u8], min_edge: u32, langs: &Languages) -> Result<RgbaImage, String> {
    if data.len() > MAX_PNG_ICON_BYTES {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-png-too-big",
            &[("kib", (MAX_PNG_ICON_BYTES >> 10).into())],
        ));
    }
    if image::guess_format(data).ok() != Some(ImageFormat::Png) {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-png-content",
            &[],
        ));
    }
    let image = decode(data, ImageFormat::Png, langs)?;
    let (w, h) = (image.width(), image.height());
    if w != h {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-not-square",
            &[("width", w.into()), ("height", h.into())],
        ));
    }
    if w < min_edge {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-png-too-small",
            &[
                ("min", min_edge.into()),
                ("width", w.into()),
                ("height", h.into()),
            ],
        ));
    }
    Ok(imageops::resize(
        &image.to_rgba8(),
        ICON_LARGE,
        ICON_LARGE,
        FilterType::Lanczos3,
    ))
}

fn svg_icon(data: &[u8], langs: &Languages) -> Result<RgbaImage, String> {
    if data.len() > MAX_SVG_ICON_BYTES {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-svg-too-big",
            &[("kib", (MAX_SVG_ICON_BYTES >> 10).into())],
        ));
    }
    let text = std::str::from_utf8(data)
        .map_err(|_| crate::l10n::text(langs, "plugin-install-img-svg-not-utf8", &[]))?;
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_XML_NODES,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(text, options).map_err(|e| {
        crate::l10n::text(
            langs,
            "plugin-install-img-svg-invalid",
            &[("error", e.to_string().into())],
        )
    })?;
    preflight::check(&doc, langs)?;
    let options = usvg::Options {
        image_href_resolver: ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_xmltree(&doc, &options).map_err(|e| {
        crate::l10n::text(
            langs,
            "plugin-install-img-svg-parse-failed",
            &[("error", e.to_string().into())],
        )
    })?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if (w - h).abs() > w.max(h) * 1e-3 {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-img-svg-viewbox",
            &[("width", w.into()), ("height", h.into())],
        ));
    }
    let mut pixmap = Pixmap::new(ICON_LARGE, ICON_LARGE)
        .ok_or_else(|| crate::l10n::text(langs, "plugin-install-img-canvas-alloc-failed", &[]))?;
    let edge = ICON_LARGE as f32;
    resvg::render(
        &tree,
        Transform::from_scale(edge / w, edge / h),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    RgbaImage::from_raw(ICON_LARGE, ICON_LARGE, rgba)
        .ok_or_else(|| crate::l10n::text(langs, "plugin-install-img-canvas-convert-failed", &[]))
}

fn decode(data: &[u8], format: ImageFormat, langs: &Languages) -> Result<DynamicImage, String> {
    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(256 << 20);
    reader.limits(limits);
    reader.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => crate::l10n::text(
            langs,
            "plugin-install-img-over-max-edge",
            &[("max", MAX_EDGE.into())],
        ),
        other => crate::l10n::text(
            langs,
            "plugin-install-img-decode-failed",
            &[("error", other.to_string().into())],
        ),
    })
}

fn encode_png(image: RgbaImage, langs: &Languages) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| {
            crate::l10n::text(
                langs,
                "plugin-install-img-encode-png-failed",
                &[("error", e.to_string().into())],
            )
        })?;
    Ok(out.into_inner())
}

/// JPEG 没有透明通道：透明的地方铺白色。
fn on_white(image: &RgbaImage) -> RgbImage {
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let [r, g, b, a] = image.get_pixel(x, y).0;
        let blend =
            |c: u8| ((u16::from(c) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8;
        image::Rgb([blend(r), blend(g), blend(b)])
    })
}

#[cfg(test)]
mod tests;
