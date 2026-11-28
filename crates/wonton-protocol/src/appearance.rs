//! App 的外观（ADR-0026、ADR-0028、ADR-0130）：用户选的明暗、字号和字体，以及浅色、深色两套黑白颜色令牌。
//!
//! 颜色只在客户端核心里定一份，壳按明暗选一套来渲染，自己不写色值。

use serde::{Deserialize, Serialize};

/// 一个 sRGB 颜色。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// 外观设置页和整个 App 的渲染都读它。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AppearanceViewModel {
    pub theme_mode: ThemeMode,
    /// 设置页能选的明暗，按显示的顺序排；名字由壳按枚举写。
    pub theme_modes: Vec<ThemeMode>,
    pub text_size: TextSize,
    /// 设置页能选的字号，按显示的顺序排。
    pub text_sizes: Vec<TextSize>,
    pub font: FontStyle,
    /// 这台设备能选的字体，按显示的顺序排：只列系统自带的，所以两端不一样。
    pub fonts: Vec<FontStyle>,
    /// 明暗是浅色时用的一套。
    pub light: ThemeColors,
    /// 明暗是深色时用的一套。
    pub dark: ThemeColors,
    /// 设置文件读不了，外观暂时用默认值。改任何一项后，文件重写，这里变空。
    pub problem: Option<AppearanceProblem>,
}

/// 浅色、深色，或者跟随系统。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

/// 字号。除了 `System`，每一档都盖过系统的字号设置，只在这个 App 里生效。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum TextSize {
    #[default]
    System,
    Small,
    Standard,
    Large,
    ExtraLarge,
}

/// 字体，都是系统自带的。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum FontStyle {
    #[default]
    Default,
    Rounded,
    Serif,
    Monospaced,
}

/// 一种明暗下的全部颜色令牌。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ThemeColors {
    pub canvas: Rgb,
    pub surface: Rgb,
    pub surface_subtle: Rgb,
    pub text_primary: Rgb,
    pub text_secondary: Rgb,
    pub separator: Rgb,
    /// 强调填充：浅色是黑，深色是比卡片浅一档的深灰。主按钮、选中、开关、角标、自己的气泡的底；
    /// 只当填充用，页面上要强调的字和图标用 `text_primary`。
    pub primary: Rgb,
    /// `primary` 底上的字和图标。
    pub on_primary: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub danger: Rgb,
}

/// 外观设置读不了的原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AppearanceProblem {
    /// 核心写的说明，带上文件路径和原因。
    pub message: String,
}
