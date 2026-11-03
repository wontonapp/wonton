//! App 的外观（ADR-0026）：用户选的重点色、由它推导的颜色令牌，和存在本机的设置文件。
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

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_protocol::{AppearanceProblem, AppearanceViewModel, CoreError, Rgb};

use crate::home::write_private;

use color::rgb;

/// 和设备密钥、连接信息同一目录、不同文件，写坏了不连累连接。
const SETTINGS_FILE: &str = "appearance.json";
const DEFAULT_ACCENT: Rgb = rgb(0xFEEBEA);
/// 第一个是默认值。
const PRESETS: [Rgb; 6] = [
    DEFAULT_ACCENT,
    rgb(0xF1ECE2),
    rgb(0xDFF1EC),
    rgb(0xE3E7FA),
    rgb(0xFFF4CC),
    rgb(0xC2410C),
];

/// 设置文件的格式。颜色存成十六进制，不直接序列化 [`Rgb`]：协议类型改字段名不会让旧文件读不了。
#[derive(Serialize, Deserialize)]
struct Settings {
    accent: String,
}

pub(crate) struct AppearanceLink {
    path: PathBuf,
    view: watch::Sender<AppearanceViewModel>,
    /// 写文件和发布新状态是一步，否则并发时文件和界面可能停在不同的颜色上。
    writing: Mutex<()>,
}

impl AppearanceLink {
    pub(crate) fn new(data_dir: &Path) -> Self {
        let path = data_dir.join(SETTINGS_FILE);
        let view = match read_accent(&path) {
            Ok(accent) => view_model(accent.unwrap_or(DEFAULT_ACCENT), None),
            Err(reason) => view_model(
                DEFAULT_ACCENT,
                Some(AppearanceProblem {
                    message: format!("外观设置读不了，先用默认颜色：{reason}"),
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

    /// 写不了时返回 `Storage`，外观不变。
    pub(crate) fn set_accent(&self, accent: Rgb) -> Result<(), CoreError> {
        let _writing = self.writing.lock().expect("写设置时不会 panic");
        let settings = Settings {
            accent: accent_hex(accent),
        };
        let text = serde_json::to_vec(&settings).expect("设置只有一个字符串字段，能序列化");
        write_private(&self.path, &text).map_err(|error| CoreError::Storage {
            reason: format!("{}：{error}", self.path.display()),
        })?;
        self.view.send_replace(view_model(accent, None));
        Ok(())
    }

    pub(crate) fn reset_accent(&self) -> Result<(), CoreError> {
        self.set_accent(DEFAULT_ACCENT)
    }
}

/// 没选过时是 `None`。
fn read_accent(path: &Path) -> Result<Option<Rgb>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}：{error}", path.display())),
    };
    let settings: Settings = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} 已损坏：{error}", path.display()))?;
    parse_accent(settings.accent.clone())
        .map(Some)
        .ok_or_else(|| format!("{} 里的颜色无效：{}", path.display(), settings.accent))
}

fn view_model(accent: Rgb, problem: Option<AppearanceProblem>) -> AppearanceViewModel {
    AppearanceViewModel {
        accent,
        presets: PRESETS.to_vec(),
        light: derive::light_theme(accent),
        dark: derive::dark_theme(accent),
        problem,
    }
}

/// 默认外观。核心起不来时启动失败页也要有颜色，所以不依赖 [`ClientCore`](crate::ClientCore)。
#[uniffi::export]
pub fn default_appearance() -> AppearanceViewModel {
    view_model(DEFAULT_ACCENT, None)
}

/// 只计算不保存：设置页拖动滑块时预览用，松手才发 `Action::SetAccentColor`。
#[uniffi::export]
pub fn preview_accent(color: Rgb) -> AppearanceViewModel {
    view_model(color, None)
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
