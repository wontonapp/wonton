//! `ui_fake` 的设置首页夹具：假机器摆成“已经配上一台家里的服务器”的样子。
//!
//! 单独成文件是因为 `ui_fake.rs` 已经过了 800 行警告线（AGENTS“代码结构”）。

use wonton_protocol::{HomeStatus, ServerName, SettingsEntry, SettingsSection, SettingsViewModel};

/// 假数据里已经配上了一台“家里”的服务器：设置页照配对后那样先给资料和服务器入口，
/// 添加服务器和“重新配对”都在服务器页里。
pub(super) fn fake_settings(mut settings: SettingsViewModel) -> SettingsViewModel {
    let mut paired = vec![
        SettingsSection {
            entries: vec![SettingsEntry::Profile],
        },
        SettingsSection {
            entries: vec![SettingsEntry::Server {
                name: Some(ServerName::Named {
                    name: "Home".into(),
                }),
                status: HomeStatus::Connected,
                upgrade: false,
            }],
        },
    ];
    paired.append(&mut settings.sections);
    settings.sections = paired;
    settings
}
