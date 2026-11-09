//! 8 位 sRGB 和 OKLCH 之间的换算，以及 WCAG 对比度。推导里的每个候选颜色都先落到 8 位再算对比度：
//! 屏幕上显示的是取整后的颜色。

use palette::convert::FromColorUnclamped;
use palette::{LinSrgb, OklabHue, Oklch, Srgb};
use wonton_protocol::Rgb;

/// 判断是否在 sRGB 色域内时的容差。沿色域边缘的颜色在浮点计算里会略微越界，不容差就会被无谓地降彩度。
const GAMUT_TOLERANCE: f64 = 1e-4;
/// 二分彩度的次数，足够让误差远小于 8 位颜色的一档。
const GAMUT_SEARCH_STEPS: u32 = 24;

pub(super) const fn rgb(hex: u32) -> Rgb {
    Rgb {
        red: (hex >> 16) as u8,
        green: (hex >> 8) as u8,
        blue: hex as u8,
    }
}

fn srgb(color: Rgb) -> Srgb<f64> {
    Srgb::new(color.red, color.green, color.blue).into_format()
}

pub(super) fn to_oklch(color: Rgb) -> Oklch<f64> {
    Oklch::from_color_unclamped(srgb(color).into_linear::<f64>())
}

/// 超出 sRGB 时只降彩度，亮度和色相不动。
pub(super) fn from_oklch(lightness: f64, chroma: f64, hue: OklabHue<f64>) -> Rgb {
    let linear = |chroma: f64| LinSrgb::from_color_unclamped(Oklch::new(lightness, chroma, hue));
    let fits = |color: LinSrgb<f64>| {
        [color.red, color.green, color.blue]
            .iter()
            .all(|channel| (-GAMUT_TOLERANCE..=1.0 + GAMUT_TOLERANCE).contains(channel))
    };
    let chroma = if fits(linear(chroma)) {
        chroma
    } else {
        let (mut low, mut high) = (0.0, chroma);
        for _ in 0..GAMUT_SEARCH_STEPS {
            let middle = (low + high) / 2.0;
            if fits(linear(middle)) {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    };
    let color = linear(chroma);
    let clamped = LinSrgb::new(
        color.red.clamp(0.0, 1.0),
        color.green.clamp(0.0, 1.0),
        color.blue.clamp(0.0, 1.0),
    );
    let bytes: Srgb<u8> = Srgb::<f64>::from_linear(clamped).into_format();
    Rgb {
        red: bytes.red,
        green: bytes.green,
        blue: bytes.blue,
    }
}

/// 按 WCAG 2.1 的公式算，不用 palette 的 `relative_contrast`：它的亮度系数由色彩空间推出，和 WCAG 写死的
/// 0.2126、0.7152、0.0722 略有出入，会把标准算出 1.4999 的颜色判成够 1.5。
pub(super) fn contrast(one: Rgb, other: Rgb) -> f64 {
    let (one, other) = (luminance(one), luminance(other));
    (one.max(other) + 0.05) / (one.min(other) + 0.05)
}

fn luminance(color: Rgb) -> f64 {
    let linear = srgb(color).into_linear::<f64>();
    0.2126 * linear.red + 0.7152 * linear.green + 0.0722 * linear.blue
}
