use serde::{Deserialize, Serialize};

use crate::{FontStyle, NotificationCategory, TextSize, ThemeMode};

/// 客户端本地外观与通知内容设置操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingsAction {
    /// App 使用的语言在第一项，后接系统语言偏好（BCP 47）；壳在启动和语言改变时发送。客户端详情用第一项，服务器按列表挑插件文字。
    SetLanguages { languages: Vec<String> },
    /// 选明暗。
    SetThemeMode { mode: ThemeMode },
    /// 选字号。
    SetTextSize { size: TextSize },
    /// 选字体，只从 [`AppearanceViewModel::fonts`](crate::AppearanceViewModel::fonts) 里选。
    SetFont { font: FontStyle },
    /// 设置指定通知类别是否在锁屏隐藏内容。
    SetLockScreenContent {
        category: NotificationCategory,
        hidden: bool,
    },
}
