use std::fs;

use wonton_protocol::{Rgb, ThemeColors};

use super::color::{contrast, rgb};
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
        let appearance = preview_accent(accent);
        assert_eq!(appearance.light.accent_fill, accent);
        assert_readable(accent, &appearance.light);
        assert_readable(accent, &appearance.dark);
    }
}

#[test]
fn chosen_color_survives_a_restart_and_reset_restores_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let chosen = rgb(0x4A47D6);
    AppearanceLink::new(dir.path()).set_accent(chosen).unwrap();
    let restarted = AppearanceLink::new(dir.path());
    assert_eq!(restarted.view(), preview_accent(chosen));

    restarted.reset_accent().unwrap();
    assert_eq!(AppearanceLink::new(dir.path()).view(), default_appearance());
}

/// 新版本要读得了旧文件。
#[test]
fn settings_file_format_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(SETTINGS_FILE), r##"{"accent":"#4A47D6"}"##).unwrap();
    assert_eq!(AppearanceLink::new(dir.path()).view().accent, rgb(0x4A47D6));
}

#[test]
fn unreadable_settings_show_the_reason_until_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(SETTINGS_FILE), "不是 JSON").unwrap();
    let link = AppearanceLink::new(dir.path());
    assert_eq!(link.view().accent, default_appearance().accent);
    assert!(link.view().problem.is_some());

    link.reset_accent().unwrap();
    assert_eq!(link.view(), default_appearance());
}
