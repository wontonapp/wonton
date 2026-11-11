//! App 的外观（ADR-0026、ADR-0028）：用户选的重点色、明暗、字号和字体，以及由重点色推导出的颜色令牌。
//!
//! 颜色只在客户端核心里算一份，壳按明暗选一套来渲染，自己不写色值。

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
    /// 用户选的重点色；没选过，或者设置读不了时，是默认值。
    pub accent: Rgb,
    /// 设置页列出的色板，第一个是默认值。
    pub presets: Vec<Rgb>,
    /// 收在“更多”里的色板。
    pub more_colors: Vec<Rgb>,
    /// 两组色板每行的个数。
    pub swatch_columns: u32,
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
    /// 主按钮的底色。
    pub primary: Rgb,
    /// 主按钮上的字和图标。
    pub on_primary: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub danger: Rgb,
    /// 重点色的填充和底色。选中状态不能只靠它表示，对页面底色的对比度可能很低。
    pub accent_fill: Rgb,
    /// 页面底色上的字和图标。
    pub accent_ink: Rgb,
    /// 填充上的图标和小标签。
    pub accent_ink_on_fill: Rgb,
    /// 填充上的正文。
    pub text_on_accent_fill: Rgb,
    /// 填充的描边；填充和页面底色分得清时为空，这时不描边。
    pub accent_fill_border: Option<Rgb>,
}

/// 外观设置读不了的原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AppearanceProblem {
    /// 核心写的说明，带上文件路径和原因。
    pub message: String,
}
