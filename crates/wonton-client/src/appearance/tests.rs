use std::fs;

use wonton_protocol::{Rgb, ThemeColors};

use super::*;

/// WCAG 2.1 的对比度。
fn contrast(one: Rgb, other: Rgb) -> f64 {
    let luminance = |color: Rgb| {
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.red) + 0.7152 * linear(color.green) + 0.0722 * linear(color.blue)
    };
    let (one, other) = (luminance(one), luminance(other));
    (one.max(other) + 0.05) / (one.min(other) + 0.05)
}

/// 字和状态色压在任何页面底色上都要读得清；`primary` 只做填充（自己的气泡、选中、主按钮），
/// 上面的 `on_primary` 要读得清，填充本身也要和页面底色分得开。深色的填充是深灰，分层主要靠阴影，
/// 所以门槛比字低，但不能低到和卡片糊在一起。
#[test]
fn both_themes_stay_readable_without_an_accent() {
    let view = default_appearance();
    for (name, theme) in [("light", view.light), ("dark", view.dark)] {
        let ThemeColors {
            canvas,
            surface,
            surface_subtle,
            ..
        } = theme;
        for page in [canvas, surface, surface_subtle] {
            for (role, ink) in [
                ("text_primary", theme.text_primary),
                ("text_secondary", theme.text_secondary),
                ("success", theme.success),
                ("warning", theme.warning),
                ("danger", theme.danger),
            ] {
                assert!(contrast(ink, page) >= 4.5, "{name} {role} on {page:?}");
            }
            assert!(
                contrast(theme.primary, page) >= 1.5,
                "{name} primary fill on {page:?}"
            );
        }
        assert!(
            contrast(theme.on_primary, theme.primary) >= 4.5,
            "{name} on_primary"
        );
    }
}

#[test]
fn choices_survive_a_restart_and_changing_one_keeps_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let link = AppearanceLink::new(dir.path());
    link.set_theme_mode(ThemeMode::Dark).unwrap();
    link.set_text_size(TextSize::Large).unwrap();
    link.set_font(FontStyle::Serif).unwrap();

    let restarted = AppearanceLink::new(dir.path()).view();
    assert_eq!(restarted.theme_mode, ThemeMode::Dark);
    assert_eq!(restarted.text_size, TextSize::Large);
    assert_eq!(restarted.font, FontStyle::Serif);
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

/// 新版本要读得了旧文件；缺的项用默认值，去掉的项（重点色时期的 `accent`）忽略，不算读坏。
#[test]
fn settings_file_format_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(SETTINGS_FILE);
    fs::write(
        &file,
        r##"{"theme_mode":"dark","accent":"#4A47D6","text_size":"extra_large","font":"monospaced"}"##,
    )
    .unwrap();
    let view = AppearanceLink::new(dir.path()).view();
    assert_eq!(view.theme_mode, ThemeMode::Dark);
    assert_eq!(view.text_size, TextSize::ExtraLarge);
    assert_eq!(view.font, FontStyle::Monospaced);
    assert_eq!(view.problem, None);

    fs::write(&file, r#"{"font":"serif"}"#).unwrap();
    let view = AppearanceLink::new(dir.path()).view();
    assert_eq!(view.font, FontStyle::Serif);
    assert_eq!(view.theme_mode, ThemeMode::System);
    assert_eq!(view.problem, None);
}

#[test]
fn unreadable_settings_show_the_reason_until_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(SETTINGS_FILE), "不是 JSON").unwrap();
    let link = AppearanceLink::new(dir.path());
    assert_eq!(link.view().font, default_appearance().font);
    assert!(link.view().problem.is_some());

    link.set_font(FontStyle::Serif).unwrap();
    assert_eq!(link.view().problem, None);
    assert_eq!(
        AppearanceLink::new(dir.path()).view().font,
        FontStyle::Serif
    );
}
