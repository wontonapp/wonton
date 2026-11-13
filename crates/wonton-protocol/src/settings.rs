//! 设置首页：分几组、每组哪几项、按什么顺序。两端都按它画，顺序和显示条件只在客户端核心写一处。
//!
//! 外部 Matrix 槽位由核心只发 `MatrixAccount`、`MatrixDevices`、`Friends`、`Notifications`、
//! `Language` 和 `AddWontonServer`，不发 Wonton 的管理入口，不让壳按槽位种类过滤。账号和设备入口针对当前
//! Matrix 账号（包括账号身份、密钥和本人的 Matrix 设备），不是 Wonton 配对设备管理。
//! `Friends` 是任意账号的好友与请求入口，和只给 Wonton 主人的服务器配对入口不同。
//! 壳不显示 `Other`：新核心增加的未知按钮不能变成空行或可点击的占位入口。

use serde::{Deserialize, Serialize};

use crate::{
    AppLockTimeout, HomeStatus, MatrixAccount, NotificationCategory, PushMethod, PushStatus,
    RecoveryStatus, ServerName,
};

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
    /// 外部 Matrix 账号的身份和账号设置，不是服务器上的 Wonton 资料。
    MatrixAccount {
        account: MatrixAccount,
    },
    /// 当前 Matrix 账号的设备、验证和密钥备份入口；不是服务器的配对设备。
    MatrixDevices,
    /// 添加自己的 Wonton，发送 `Action::AddServer`，不把外部账号变成 Wonton。
    AddWontonServer,
    /// 任意账号的好友和好友请求，不要求拥有或配对 Wonton。
    Friends {
        /// 已成为好友的人数。
        count: u32,
        /// 收到的、尚未处理的请求数，不含发出的请求。
        requests: u32,
    },
    /// 家里的服务器：连接状态，以及出网代理、升级、添加设备、重新配对。配对以后才有。
    Server {
        /// 配了不止一台时是当前这台的名字，壳和状态并排显示；只有一台时为空。
        name: Option<ServerName>,
        status: HomeStatus,
        /// 服务器有新版本、主人还没跳过，画角标。
        upgrade: bool,
    },
    /// 明暗、重点色、字号和字体。
    Appearance,
    /// App 的显示语言，壳按平台打开系统的语言设置或自己的选择器。
    Language,
    /// 其他服务器：配对过的 Matrix 服务器（ADR-0050）。只给主人。
    Peers {
        /// 已配对的台数。
        count: u32,
    },
    /// 我的应用：放哪些、顺序、新装应用自动加入。
    MyApps,
    /// 通知：推送收不收得到，以及按类别的开关（开关在系统设置里，Android 是每个类别的通知渠道）。
    Notifications {
        method: PushMethod,
        push: PushStatus,
        categories: Vec<NotificationCategory>,
    },
    /// App 锁（ADR-0102）：开关和退到后台后多久要求解锁。
    AppLock {
        enabled: bool,
        timeout: AppLockTimeout,
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
    /// 比壳新的核心给的未知入口，壳必须忽略，不显示按钮或占位行。
    #[serde(other)]
    Other,
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
                            name: Some(ServerName::Named {
                                name: "办公室".into(),
                            }),
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

    #[test]
    fn external_account_settings_round_trip_without_server_management_entries() {
        let view = SettingsViewModel {
            sections: vec![
                SettingsSection {
                    entries: vec![
                        SettingsEntry::MatrixAccount {
                            account: MatrixAccount {
                                user_id: "@alice:matrix.org".into(),
                                homeserver: "https://matrix-client.matrix.org/".into(),
                            },
                        },
                        SettingsEntry::MatrixDevices,
                        SettingsEntry::Friends {
                            count: 2,
                            requests: 1,
                        },
                    ],
                },
                SettingsSection {
                    entries: vec![
                        SettingsEntry::Notifications {
                            method: PushMethod::Keepalive,
                            push: PushStatus::NoEndpoint,
                            categories: vec![],
                        },
                        SettingsEntry::Language,
                        SettingsEntry::AddWontonServer,
                    ],
                },
            ],
        };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(
            serde_json::from_value::<SettingsViewModel>(json).unwrap(),
            view
        );
        assert_eq!(
            serde_json::from_str::<SettingsEntry>(r#"{"type":"future_button","url":"ignored"}"#)
                .unwrap(),
            SettingsEntry::Other
        );
        let json = serde_json::to_value(SettingsEntry::Other).unwrap();
        assert_eq!(
            serde_json::from_value::<SettingsEntry>(json).unwrap(),
            SettingsEntry::Other
        );
    }
}
