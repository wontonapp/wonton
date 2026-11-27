//! App 的外观（ADR-0026、ADR-0028、ADR-0130）：用户选的明暗、字号和字体，浅色、深色两套颜色令牌，和存在本机的设置文件。
//!
//! 设置文件读不了不影响打开 App：外观用默认值并带上原因，用户进到设置页能重设。

#[cfg(test)]
mod tests;
mod themes;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{
    AppearanceProblem, AppearanceViewModel, CoreError, FontStyle, TextSize, ThemeMode,
};

use crate::home::write_private;
use crate::l10n::{CATALOG, language};

pub(crate) use themes::hex;

/// 和设备密钥、连接信息同一目录、不同文件，写坏了不连累连接。
const SETTINGS_FILE: &str = "appearance.json";
/// 用户的选择。
#[derive(Clone, Copy, Default)]
struct Choices {
    theme_mode: ThemeMode,
    text_size: TextSize,
    font: FontStyle,
}

/// 设置文件的格式。
#[derive(Serialize, Deserialize)]
struct Settings {
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
                theme_mode: view.theme_mode,
                text_size: view.text_size,
                font: view.font,
            }
        };
        apply(&mut choices);
        let settings = Settings {
            theme_mode: choices.theme_mode,
            text_size: choices.text_size,
            font: choices.font,
        };
        let text = serde_json::to_vec(&settings).expect("settings only have enum fields");
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
    Ok(Some(Choices {
        theme_mode: settings.theme_mode,
        text_size: settings.text_size,
        font: settings.font,
    }))
}

fn view_model(choices: Choices, problem: Option<AppearanceProblem>) -> AppearanceViewModel {
    AppearanceViewModel {
        theme_mode: choices.theme_mode,
        theme_modes: theme_modes(),
        text_size: choices.text_size,
        text_sizes: text_sizes(),
        font: choices.font,
        fonts: fonts(),
        light: themes::LIGHT,
        dark: themes::DARK,
        problem,
    }
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
