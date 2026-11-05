// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
use wonton_protocol::{Rgb, ThemeColors};

type ColorVariable = (&'static str, fn(&ThemeColors) -> Rgb);

pub(crate) const COLOR_VARIABLES: &[ColorVariable] = &[
    ("--color-background-primary", |c| c.canvas),
    ("--color-background-secondary", |c| c.surface_subtle),
    ("--color-text-primary", |c| c.text_primary),
    ("--color-text-secondary", |c| c.text_secondary),
    ("--color-border-primary", |c| c.separator),
    ("--color-background-danger", |c| c.danger),
    ("--color-text-danger", |c| c.danger),
    ("--color-border-danger", |c| c.danger),
    ("--color-ring-danger", |c| c.danger),
    ("--color-background-success", |c| c.success),
    ("--color-text-success", |c| c.success),
    ("--color-border-success", |c| c.success),
    ("--color-ring-success", |c| c.success),
    ("--color-background-warning", |c| c.warning),
    ("--color-text-warning", |c| c.warning),
    ("--color-border-warning", |c| c.warning),
    ("--color-ring-warning", |c| c.warning),
    ("--wonton-accent-fill", |c| c.accent_fill),
    ("--wonton-accent-ink", |c| c.accent_ink),
    ("--wonton-accent-ink-on-fill", |c| c.accent_ink_on_fill),
];

pub(crate) const RADIUS_VARIABLES: &[(&str, f64, f64)] = &[
    ("--border-radius-xs", 6.0, 4.0),
    ("--border-radius-md", 14.0, 12.0),
    ("--border-radius-lg", 20.0, 16.0),
    ("--border-radius-full", 999.0, 999.0),
];

pub(crate) const FONT_VARIABLE: &str = "--font-sans";
