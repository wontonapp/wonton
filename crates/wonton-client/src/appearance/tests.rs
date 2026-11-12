use std::fs;

use wonton_protocol::{Rgb, ThemeColors};

use super::color::{contrast, rgb};
use super::derive;
use super::*;

fn assert_readable(accent: Rgb, theme: &ThemeColors) {
    let pages = [theme.canvas, theme.surface, theme.surface_subtle];
    for page in pages {
        assert!(contrast(theme.accent_ink, page) >= 5.5, "{accent:?}");
    }
    assert!(
        contrast(theme.accent_ink_on_fill, theme.accent_fill) >= 4.5,
        "{accent:?}"
    );
    assert!(
        contrast(theme.text_on_accent_fill, theme.accent_fill) >= 4.5,
        "{accent:?}"
    );
    let needs_border = pages
        .iter()
        .any(|&page| contrast(theme.accent_fill, page) < 1.5);
    assert_eq!(
        theme.accent_fill_border.is_some(),
        needs_border,
        "{accent:?}"
    );
    if let Some(border) = theme.accent_fill_border {
        for page in pages {
            assert!(contrast(border, page) >= 1.5, "{accent:?}");
        }
    }
}

/// 用户能选任意 RGB，这是唯一能保证选什么都读得清的地方。网格之外再加容易出错的边界：接近灰色和接近白色。
#[test]
fn any_accent_stays_readable() {
    let grid = (0..=255u32).step_by(17);
    let colors = grid.clone().flat_map(|red| {
        grid.clone().flat_map({
            let grid = grid.clone();
            move |green| grid.clone().map(move |blue| red << 16 | green << 8 | blue)
        })
    });
    let edges = [
        0xFFFFFE, 0xFFFDFD, 0x010000, 0x7F7F80, 0x3A3A3F, 0xC7C4C3, 0x95766C,
    ];
    for accent in colors.chain(edges).map(rgb) {
        let light = derive::light_theme(accent);
        assert_eq!(light.accent_fill, accent);
        assert_readable(accent, &light);
        assert_readable(accent, &derive::dark_theme(accent));
    }
}

#[test]
fn choices_survive_a_restart_and_changing_one_keeps_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let link = AppearanceLink::new(dir.path());
    let chosen = rgb(0x4A47D6);
    link.set_accent(chosen).unwrap();
    link.set_theme_mode(ThemeMode::Dark).unwrap();
    link.set_text_size(TextSize::Large).unwrap();
    link.set_font(FontStyle::Serif).unwrap();

    let restarted = AppearanceLink::new(dir.path()).view();
    assert_eq!(restarted.accent, chosen);
    assert_eq!(restarted.theme_mode, ThemeMode::Dark);
    assert_eq!(restarted.text_size, TextSize::Large);
    assert_eq!(restarted.font, FontStyle::Serif);
    assert_eq!(
        restarted.dark.accent_fill,
        derive::dark_theme(chosen).accent_fill
    );
}

/// 选项只是枚举，顺序由核心定，名字由壳写。只有 Apple 的系统字体里有圆体。
#[test]
fn option_lists_are_enums_in_display_order() {
    let view = default_appearance();
    assert_eq!(
        view.theme_modes,
        [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark]
    );
    assert_eq!(view.text_sizes.first(), Some(&TextSize::System));
    assert_eq!(view.fonts.first(), Some(&FontStyle::Default));
    assert_eq!(
        view.fonts.contains(&FontStyle::Rounded),
        cfg!(any(target_os = "ios", target_os = "macos"))
    );
}

/// 新版本要读得了旧文件。
#[test]
fn settings_file_format_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(SETTINGS_FILE);
    fs::write(
        &file,
        r##"{"accent":"#4A47D6","theme_mode":"dark","text_size":"extra_large","font":"monospaced"}"##,
    )
    .unwrap();
    let view = AppearanceLink::new(dir.path()).view();
    assert_eq!(view.accent, rgb(0x4A47D6));
    assert_eq!(view.theme_mode, ThemeMode::Dark);
    assert_eq!(view.text_size, TextSize::ExtraLarge);
    assert_eq!(view.font, FontStyle::Monospaced);

    fs::write(&file, r##"{"accent":"#4A47D6"}"##).unwrap();
    let view = AppearanceLink::new(dir.path()).view();
    assert_eq!(view.accent, rgb(0x4A47D6));
    assert_eq!(view.theme_mode, ThemeMode::System);
    assert_eq!(view.problem, None);
}

#[test]
fn unreadable_settings_show_the_reason_until_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(SETTINGS_FILE), "不是 JSON").unwrap();
    let link = AppearanceLink::new(dir.path());
    assert_eq!(link.view().accent, default_appearance().accent);
    assert!(link.view().problem.is_some());

    link.set_font(FontStyle::Serif).unwrap();
    assert_eq!(link.view().problem, None);
    assert_eq!(
        AppearanceLink::new(dir.path()).view().font,
        FontStyle::Serif
    );
}
