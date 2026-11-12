//! 设置首页：分几组、每组哪几项、按什么顺序。两端都按它画，顺序和显示条件只在客户端核心写一处。

use serde::{Deserialize, Serialize};

use crate::{HomeStatus, NotificationCategory, PushStatus, RecoveryStatus};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettingsViewModel {
    /// 按显示顺序；每组画成一个分组，组里一项一行。
    pub sections: Vec<SettingsSection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettingsSection {
    pub entries: Vec<SettingsEntry>,
}

/// 设置首页的一行。标题、图标和点了去哪由壳按种类定，值由核心给。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingsEntry {
    /// 你的资料。资料存在服务器上，配对以后才有。
    Profile,
    /// 家里的服务器：连接状态，以及出网代理、升级、添加设备、重新配对。配对以后才有。
    Server {
        /// 配了不止一台时是当前这台的名字，壳和状态并排显示；只有一台时为空。
        name: Option<String>,
        status: HomeStatus,
        /// 服务器有新版本、主人还没跳过，画角标。
        upgrade: bool,
    },
    /// 明暗、重点色、字号和字体。
    Appearance,
    /// 底栏的项、顺序和是否显示文字。
    BottomBar,
    /// 通知：推送收不收得到，以及按类别的开关（开关在系统设置里，Android 是每个类别的通知渠道）。
    Notifications {
        push: PushStatus,
        categories: Vec<NotificationCategory>,
    },
    /// 默认 Agent。配对以后才有。
    DefaultAgent,
    ChatBackup {
        recovery: RecoveryStatus,
    },
    /// 再走一遍引导。配对以后才有。
    RestartOnboarding,
    /// 关于这个 App，比如版本号。
    About,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_model_round_trips_through_json() {
        let view = SettingsViewModel {
            sections: vec![
                SettingsSection {
                    entries: vec![SettingsEntry::Profile],
                },
                SettingsSection {
                    entries: vec![
                        SettingsEntry::Server {
                            name: Some("办公室".into()),
                            status: HomeStatus::Connected,
                            upgrade: true,
                        },
                        SettingsEntry::ChatBackup {
                            recovery: RecoveryStatus::Enabled,
                        },
                    ],
                },
            ],
        };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(
            serde_json::from_value::<SettingsViewModel>(json).unwrap(),
            view
        );
    }
}
