//! 由一个重点色推导两种明暗下的全部颜色令牌（ADR-0026）。

use palette::OklabHue;
use wonton_protocol::{Rgb, ThemeColors};

use super::color::{contrast, from_oklch, rgb, to_oklch};

/// 填充上的字和图标的最低对比度（WCAG AA）。
const MIN_CONTRAST: f64 = 4.5;
/// 页面上的 `accentInk` 的最低对比度。比 AA 高一档，叠了噪点、字又小时仍然读得清。
const INK_CONTRAST: f64 = 5.5;
const BORDER_CONTRAST: f64 = 1.5;
/// 彩度低于它的颜色按比例收向灰：肉眼看不出差别的两个颜色，结果也看不出差别。
const CHROMA_RAMP: f64 = 0.02;
/// 很浅的颜色彩度太低，压暗后会变成灰色，所以推导出的 `accentInk` 不低于这个彩度。
const INK_MIN_CHROMA: f64 = 0.11;
const FILL_DARK_LIGHTNESS: f64 = 0.31;
const FILL_DARK_MIN_CHROMA: f64 = 0.035;
const FILL_DARK_MAX_CHROMA: f64 = 0.09;
const INK_LIGHT_START: f64 = 0.62;
const INK_DARK_START: f64 = 0.72;
const BORDER_LIGHT_START: f64 = 0.98;
const BORDER_DARK_START: f64 = 0.2;
const LIGHTNESS_STEP: f64 = 0.005;

const BLACK: Rgb = rgb(0x000000);
const WHITE: Rgb = rgb(0xFFFFFF);

const LIGHT: ThemeColors = neutrals(
    [0xFFFFFF, 0xFFFFFF, 0xF4F4F4, 0x111111, 0x5E5E5E, 0xE4E4E4],
    [0x111111, 0xFFFFFF, 0x18753C, 0x8A4B00, 0xB42318],
);
const DARK: ThemeColors = neutrals(
    [0x0E0E0E, 0x191919, 0x262626, 0xF5F5F5, 0xABABAB, 0x333333],
    [0xF5F5F5, 0x111111, 0x7CDCA0, 0xFFC379, 0xFFA69F],
);

const fn neutrals(page: [u32; 6], controls: [u32; 5]) -> ThemeColors {
    let [
        canvas,
        surface,
        surface_subtle,
        text_primary,
        text_secondary,
        separator,
    ] = page;
    let [primary, on_primary, success, warning, danger] = controls;
    ThemeColors {
        canvas: rgb(canvas),
        surface: rgb(surface),
        surface_subtle: rgb(surface_subtle),
        text_primary: rgb(text_primary),
        text_secondary: rgb(text_secondary),
        separator: rgb(separator),
        primary: rgb(primary),
        on_primary: rgb(on_primary),
        success: rgb(success),
        warning: rgb(warning),
        danger: rgb(danger),
        // 下面几个由 `theme` 按用户的颜色填上。
        accent_fill: rgb(0),
        accent_ink: rgb(0),
        accent_ink_on_fill: rgb(0),
        text_on_accent_fill: rgb(0),
        accent_fill_border: None,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Light,
    Dark,
}

pub(super) fn light_theme(accent: Rgb) -> ThemeColors {
    theme(LIGHT, accent, Mode::Light)
}

pub(super) fn dark_theme(accent: Rgb) -> ThemeColors {
    theme(DARK, accent, Mode::Dark)
}

fn theme(neutrals: ThemeColors, accent: Rgb, mode: Mode) -> ThemeColors {
    let pages = [neutrals.canvas, neutrals.surface, neutrals.surface_subtle];
    let oklch = to_oklch(accent);
    let (lightness, chroma, hue) = (oklch.l, oklch.chroma, oklch.hue);
    let ramp = (chroma / CHROMA_RAMP).min(1.0);
    let derived_chroma = ramp * chroma.max(INK_MIN_CHROMA);

    let accent_fill = match mode {
        Mode::Light => accent,
        Mode::Dark => from_oklch(
            FILL_DARK_LIGHTNESS,
            ramp * chroma.clamp(FILL_DARK_MIN_CHROMA, FILL_DARK_MAX_CHROMA),
            hue,
        ),
    };
    let search = Search {
        hue,
        chroma: derived_chroma,
        pages,
        mode,
    };
    let accent_ink = if pages
        .iter()
        .all(|&page| contrast(accent, page) >= INK_CONTRAST)
    {
        accent
    } else {
        let start = match mode {
            Mode::Light => lightness.min(INK_LIGHT_START),
            Mode::Dark => lightness.max(INK_DARK_START),
        };
        search.readable(start, INK_CONTRAST)
    };
    let text_on_accent_fill = if contrast(neutrals.text_primary, accent_fill) >= MIN_CONTRAST {
        neutrals.text_primary
    } else if contrast(BLACK, accent_fill) >= contrast(WHITE, accent_fill) {
        BLACK
    } else {
        WHITE
    };
    let accent_ink_on_fill = if contrast(accent_ink, accent_fill) >= MIN_CONTRAST {
        accent_ink
    } else {
        text_on_accent_fill
    };
    let accent_fill_border = pages
        .iter()
        .any(|&page| contrast(accent_fill, page) < BORDER_CONTRAST)
        .then(|| {
            let start = match mode {
                Mode::Light => BORDER_LIGHT_START,
                Mode::Dark => BORDER_DARK_START,
            };
            search.readable(start, BORDER_CONTRAST)
        });
    ThemeColors {
        accent_fill,
        accent_ink,
        accent_ink_on_fill,
        text_on_accent_fill,
        accent_fill_border,
        ..neutrals
    }
}

/// 保持色相和彩度，沿亮度找颜色：浅色模式往深，深色模式往浅。
struct Search {
    hue: OklabHue<f64>,
    chroma: f64,
    pages: [Rgb; 3],
    mode: Mode,
}

impl Search {
    /// 从 `start` 开始，离它最近、对所有页面底色都达到 `target` 的一档。
    fn readable(&self, start: f64, target: f64) -> Rgb {
        let (direction, end) = match self.mode {
            Mode::Light => (-1.0, BLACK),
            Mode::Dark => (1.0, WHITE),
        };
        let lightnesses = (0u32..)
            .map(|step| start + direction * LIGHTNESS_STEP * f64::from(step))
            .take_while(|lightness| (0.0..=1.0).contains(lightness));
        for lightness in lightnesses {
            let color = from_oklch(lightness, self.chroma, self.hue);
            if self
                .pages
                .iter()
                .all(|&page| contrast(color, page) >= target)
            {
                return color;
            }
        }
        end
    }
}
