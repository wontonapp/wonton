//! 浅色、深色两套颜色令牌（ADR-0130）。界面只有黑白灰和三种状态色，强调靠 `primary` 填充：浅色是黑底白字；
//! 深色是比卡片浅一档的深灰底白字，层次靠阴影。`primary` 只当填充用，页面上的字和图标用 `text_primary`。

use wonton_protocol::{Rgb, ThemeColors};

pub(super) const LIGHT: ThemeColors = theme(
    [0xFFFFFF, 0xFFFFFF, 0xF4F4F4, 0x111111, 0x5E5E5E, 0xE4E4E4],
    [0x111111, 0xFFFFFF, 0x18753C, 0x8A4B00, 0xB42318],
);

pub(super) const DARK: ThemeColors = theme(
    [0x0E0E0E, 0x191919, 0x262626, 0xF5F5F5, 0xABABAB, 0x333333],
    [0x454545, 0xFFFFFF, 0x7CDCA0, 0xFFC379, 0xFFA69F],
);

const fn theme(page: [u32; 6], controls: [u32; 5]) -> ThemeColors {
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
    }
}

const fn rgb(hex: u32) -> Rgb {
    Rgb {
        red: (hex >> 16) as u8,
        green: (hex >> 8) as u8,
        blue: hex as u8,
    }
}

/// `#RRGGBB`，大写。插件网页的 CSS 变量用它。
pub(crate) fn hex(color: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", color.red, color.green, color.blue)
}
