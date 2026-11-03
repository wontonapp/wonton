//! App 的外观（ADR-0026）：用户选的重点色和由它推导出的颜色令牌。
//!
//! 颜色只在客户端核心里算一份，壳按系统的明暗选一套来渲染，自己不写色值。

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
    /// 设置页列出的预设颜色，第一个是默认值。
    pub presets: Vec<Rgb>,
    /// 系统是浅色时用的一套。
    pub light: ThemeColors,
    /// 系统是深色时用的一套。
    pub dark: ThemeColors,
    /// 设置文件读不了，外观暂时用默认值。选一个颜色或恢复默认后，文件重写，这里变空。
    pub problem: Option<AppearanceProblem>,
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
