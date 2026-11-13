use serde_json::{Map, Value, json};
use wonton_l10n::l10n;
use wonton_protocol::{AppearanceViewModel, CoreError, FontStyle};

use super::tokens::{COLOR_VARIABLES, FONT_VARIABLE, RADIUS_VARIABLES};
use super::{TabEnvironment, TabPlatform};
use crate::l10n::{CATALOG, language};

pub(super) fn payload(
    appearance: &AppearanceViewModel,
    environment: &TabEnvironment,
) -> Result<Value, CoreError> {
    validate_environment(environment)?;
    let colors = if environment.dark {
        &appearance.dark
    } else {
        &appearance.light
    };
    let mut variables = Map::new();
    for (name, color) in COLOR_VARIABLES {
        variables.insert(
            (*name).to_owned(),
            crate::appearance::accent_hex(color(colors)).into(),
        );
    }
    for (name, ios, android) in RADIUS_VARIABLES {
        let radius = match environment.platform {
            TabPlatform::Ios => ios,
            TabPlatform::Android => android,
        };
        variables.insert((*name).to_owned(), format!("{radius}px").into());
    }
    variables.insert(
        FONT_VARIABLE.to_owned(),
        font_family(appearance.font).into(),
    );
    let standard: Map<String, Value> = variables
        .iter()
        .filter(|(name, _)| !name.starts_with("--wonton-"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    // 整页铺满整个屏幕（ADR-0095）：不论 `layout`，页面都被状态栏和 Home 条挡着，变量照实给。
    let insets = [
        ("top", environment.inset_top),
        ("right", environment.safe_area.right),
        ("bottom", environment.inset_bottom),
        ("left", environment.safe_area.left),
    ];
    for (side, value) in insets {
        variables.insert(
            format!("--wonton-inset-{side}"),
            format!("{value}px").into(),
        );
    }
    Ok(json!({
        "variables": variables,
        "context": {
            "theme": if environment.dark { "dark" } else { "light" },
            "styles": { "variables": standard },
            "locale": environment.locale,
            "timeZone": environment.time_zone,
            "platform": "mobile",
            "displayMode": "fullscreen",
            "availableDisplayModes": ["inline", "fullscreen"],
            "safeAreaInsets": environment.safe_area,
        },
    }))
}

fn font_family(font: FontStyle) -> &'static str {
    match font {
        FontStyle::Default => "system-ui, sans-serif",
        FontStyle::Rounded => "ui-rounded, system-ui, sans-serif",
        FontStyle::Serif => "ui-serif, serif",
        FontStyle::Monospaced => "ui-monospace, monospace",
    }
}

fn validate_environment(environment: &TabEnvironment) -> Result<(), CoreError> {
    let safe = environment.safe_area;
    if [
        environment.inset_top,
        environment.inset_bottom,
        safe.top,
        safe.right,
        safe.bottom,
        safe.left,
    ]
    .iter()
    .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(CoreError::InvalidInput {
            reason: l10n!(CATALOG, language(), "tabs-bridge-invalid-environment"),
        });
    }
    Ok(())
}
