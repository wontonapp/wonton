//! App 的外观（ADR-0026、ADR-0028）：用户选的重点色、明暗、字号和字体，由重点色推导的颜色令牌，和存在本机的设置文件。
//!
//! 设置文件读不了不影响打开 App：外观用默认值并带上原因，用户进到设置页能重设。

mod color;
mod derive;
#[cfg(test)]
mod tests;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use palette::OklabHue;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{
    AppearanceProblem, AppearanceViewModel, CoreError, FontStyle, Rgb, TextSize, ThemeMode,
};

use crate::home::write_private;
use crate::l10n::{CATALOG, language};

use color::{from_oklch, rgb};

/// 和设备密钥、连接信息同一目录、不同文件，写坏了不连累连接。
const SETTINGS_FILE: &str = "appearance.json";
const DEFAULT_ACCENT: Rgb = rgb(0xFEEBEA);
/// 第一个是默认值。前面是纸感的浅色，后面是压得住的深色。
const PRESETS: [Rgb; 12] = [
    DEFAULT_ACCENT,
    rgb(0xF1ECE2),
    rgb(0xFFF4CC),
    rgb(0xE6F2D5),
    rgb(0xDFF1EC),
    rgb(0xD9EEF9),
    rgb(0xE3E7FA),
    rgb(0xF6E0F5),
    rgb(0xC2410C),
    rgb(0x2F6B5E),
    rgb(0x3D4FA8),
    rgb(0x8E2F5C),
];
const SWATCH_COLUMNS: usize = 6;
/// “更多”色板的每一列是一个色相，每一行是一档（亮度，彩度）。
const MORE_HUES: [f64; SWATCH_COLUMNS] = [25.0, 85.0, 150.0, 210.0, 270.0, 330.0];
const MORE_TONES: [(f64, f64); 6] = [
    (0.94, 0.03),
    (0.87, 0.07),
    (0.76, 0.12),
    (0.62, 0.15),
    (0.48, 0.13),
    (0.34, 0.09),
];

/// 用户的选择。
#[derive(Clone, Copy)]
struct Choices {
    accent: Rgb,
    theme_mode: ThemeMode,
    text_size: TextSize,
    font: FontStyle,
}

impl Default for Choices {
    fn default() -> Self {
        Self {
            accent: DEFAULT_ACCENT,
            theme_mode: ThemeMode::default(),
            text_size: TextSize::default(),
            font: FontStyle::default(),
        }
    }
}

/// 设置文件的格式。颜色存成十六进制，不直接序列化 [`Rgb`]：协议类型改字段名不会让旧文件读不了。
#[derive(Serialize, Deserialize)]
struct Settings {
    accent: String,
    #[serde(default)]
    theme_mode: ThemeMode,
    #[serde(default)]
    text_size: TextSize,
    #[serde(default)]
    font: FontStyle,
}

pub(crate) struct AppearanceLink {
    path: PathBuf,
    view: watch::Sender<AppearanceViewModel>,
    /// 写文件和发布新状态是一步，否则并发时文件和界面可能停在不同的设置上。
    writing: Mutex<()>,
}

impl AppearanceLink {
    pub(crate) fn new(data_dir: &Path) -> Self {
        let path = data_dir.join(SETTINGS_FILE);
        let view = match read_choices(&path) {
            Ok(choices) => view_model(choices.unwrap_or_default(), None),
            Err(reason) => view_model(
                Choices::default(),
                Some(AppearanceProblem {
                    message: l10n!(
                        CATALOG,
                        language(),
                        "appearance-settings-unreadable",
                        reason = reason
                    ),
                }),
            ),
        };
        Self {
            path,
            view: watch::Sender::new(view),
            writing: Mutex::new(()),
        }
    }

    pub(crate) fn view(&self) -> AppearanceViewModel {
        self.view.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<AppearanceViewModel> {
        self.view.subscribe()
    }

    pub(crate) fn set_accent(&self, accent: Rgb) -> Result<(), CoreError> {
        self.change(|choices| choices.accent = accent)
    }

    pub(crate) fn set_theme_mode(&self, theme_mode: ThemeMode) -> Result<(), CoreError> {
        self.change(|choices| choices.theme_mode = theme_mode)
    }

    pub(crate) fn set_text_size(&self, text_size: TextSize) -> Result<(), CoreError> {
        self.change(|choices| choices.text_size = text_size)
    }

    pub(crate) fn set_font(&self, font: FontStyle) -> Result<(), CoreError> {
        self.change(|choices| choices.font = font)
    }

    /// 写不了时返回 `Storage`，外观不变。
    fn change(&self, apply: impl FnOnce(&mut Choices)) -> Result<(), CoreError> {
        let _writing = self.writing.lock().expect("the writing lock never panics");
        let mut choices = {
            let view = self.view.borrow();
            Choices {
                accent: view.accent,
                theme_mode: view.theme_mode,
                text_size: view.text_size,
                font: view.font,
            }
        };
        apply(&mut choices);
        let settings = Settings {
            accent: accent_hex(choices.accent),
            theme_mode: choices.theme_mode,
            text_size: choices.text_size,
            font: choices.font,
        };
        let text = serde_json::to_vec(&settings).expect("settings only have string fields");
        write_private(&self.path, &text).map_err(|error| CoreError::Storage {
            reason: l10n!(
                CATALOG,
                language(),
                "appearance-settings-write-failed",
                path = self.path.display().to_string(),
                reason = error.to_string()
            ),
        })?;
        self.view.send_replace(view_model(choices, None));
        Ok(())
    }
}

/// 没选过时是 `None`。
fn read_choices(path: &Path) -> Result<Option<Choices>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(l10n!(
                CATALOG,
                language(),
                "appearance-settings-read-failed",
                path = path.display().to_string(),
                reason = error.to_string()
            ));
        }
    };
    let settings: Settings = serde_json::from_slice(&bytes).map_err(|error| {
        l10n!(
            CATALOG,
            language(),
            "appearance-settings-corrupt",
            path = path.display().to_string(),
            reason = error.to_string()
        )
    })?;
    let accent = parse_accent(settings.accent.clone()).ok_or_else(|| {
        l10n!(
            CATALOG,
            language(),
            "appearance-settings-invalid-accent",
            path = path.display().to_string(),
            value = settings.accent
        )
    })?;
    Ok(Some(Choices {
        accent,
        theme_mode: settings.theme_mode,
        text_size: settings.text_size,
        font: settings.font,
    }))
}

fn view_model(choices: Choices, problem: Option<AppearanceProblem>) -> AppearanceViewModel {
    AppearanceViewModel {
        accent: choices.accent,
        presets: PRESETS.to_vec(),
        more_colors: more_colors(),
        swatch_columns: SWATCH_COLUMNS as u32,
        theme_mode: choices.theme_mode,
        theme_modes: theme_modes(),
        text_size: choices.text_size,
        text_sizes: text_sizes(),
        font: choices.font,
        fonts: fonts(),
        light: derive::light_theme(choices.accent),
        dark: derive::dark_theme(choices.accent),
        problem,
    }
}

fn more_colors() -> Vec<Rgb> {
    MORE_TONES
        .iter()
        .flat_map(|&(lightness, chroma)| {
            MORE_HUES
                .iter()
                .map(move |&hue| from_oklch(lightness, chroma, OklabHue::new(hue)))
        })
        .collect()
}

fn theme_modes() -> Vec<ThemeMode> {
    vec![ThemeMode::System, ThemeMode::Light, ThemeMode::Dark]
}

fn text_sizes() -> Vec<TextSize> {
    vec![
        TextSize::System,
        TextSize::Small,
        TextSize::Standard,
        TextSize::Large,
        TextSize::ExtraLarge,
    ]
}

fn fonts() -> Vec<FontStyle> {
    let mut fonts = vec![FontStyle::Default];
    // 只有 Apple 的系统字体里有圆体。
    if cfg!(any(target_os = "ios", target_os = "macos")) {
        fonts.push(FontStyle::Rounded);
    }
    fonts.extend([FontStyle::Serif, FontStyle::Monospaced]);
    fonts
}

/// 默认外观。核心起不来时启动失败页也要有颜色，所以不依赖 [`ClientCore`](crate::ClientCore)。
#[uniffi::export]
pub fn default_appearance() -> AppearanceViewModel {
    view_model(Choices::default(), None)
}

/// 解析用户输入的 `#RRGGBB`，井号可以省略，大小写不限。不是这个格式时返回空。
#[uniffi::export]
pub fn parse_accent(text: String) -> Option<Rgb> {
    let digits = text.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(rgb)
}

/// `#RRGGBB`，大写。
#[uniffi::export]
pub fn accent_hex(color: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", color.red, color.green, color.blue)
}
