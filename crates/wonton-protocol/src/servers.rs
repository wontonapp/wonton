//! 服务器槽位（ADR-0042、ADR-0051）：同一时间用一个槽位，账号的数据和会话各自隔离。
//!
//! 配对过的 Wonton 和外部 Matrix 账号并列；一个外部账号一个槽位，完整用户 ID 是账号身份，
//! 槽位 ID 只在本机引用它。Matrix 账号没有 Wonton endpoint，不得用 homeserver 或用户 ID 伪造。
//! 登录成功前保留原槽位；取消或失败不覆盖它的数据，成功后才持久化新槽位并切换。
//!
//! 导航能力由客户端核心通过 `TabsViewModel.items`、`SettingsViewModel.sections` 和插件列表给出，
//! 壳不根据 `ServerKind` 隐藏入口。外部槽位底栏只给聊天、应用，应用只列朋友分享的；设置只给
//! 账号、设备、通知、语言和添加 Wonton 的入口，不发服务器管理、家人、Bot、Agent 或商店入口。

use serde::{Deserialize, Serialize};

use crate::{MatrixAccount, MatrixLoginViewModel};

/// 配过的服务器和正在连的是哪一台。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ServersViewModel {
    /// 已配对的 Wonton 和已登录的 Matrix 账号，按添加的顺序。
    pub servers: Vec<ServerRow>,
    /// 正在添加 Wonton：当前连的是一个还没配对的新位置，壳显示配对页，配对页上可以
    /// [`Action::CancelAddServer`](crate::Action::CancelAddServer) 回到原来那台。
    pub adding: bool,
    /// 外部 Matrix 登录进展；由同一个服务器列表订阅传递，不替换当前槽位的 ViewModel。
    pub matrix_login: Option<MatrixLoginViewModel>,
}

/// 主人起的名字，或由壳按编号和 App 语言显示的默认名。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerName {
    Named { name: String },
    Default { number: u32 },
}

/// 一个已配对的 Wonton 或已登录的外部 Matrix 账号。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ServerRow {
    /// 这台服务器在本机的 ID。操作和通知都用它引用服务器，和服务器自己的 endpoint ID 不是一回事。
    pub id: String,
    pub name: ServerName,
    pub kind: ServerKind,
    /// 正在连的就是这台。
    pub current: bool,
}

/// 槽位的身份，不是壳决定导航能力的开关。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerKind {
    /// 配对过的 Wonton，连接信息和角色仍由它的服务器作用域提供。
    Wonton,
    /// 标准 Matrix 登录得到的账号，没有配对隧道 endpoint。
    Matrix { account: MatrixAccount },
    /// 未知槽位种类；保留列表行，不推断可用能力。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Action;

    /// 契约：ViewModel 和切换服务器的操作都能经 JSON 往返。
    #[test]
    fn servers_and_their_actions_round_trip_through_json() {
        let view = ServersViewModel {
            servers: vec![
                ServerRow {
                    id: "a1".into(),
                    name: ServerName::Default { number: 1 },
                    kind: ServerKind::Wonton,
                    current: true,
                },
                ServerRow {
                    id: "b2".into(),
                    name: ServerName::Named {
                        name: "办公室".into(),
                    },
                    kind: ServerKind::Matrix {
                        account: MatrixAccount {
                            user_id: "@alice:matrix.org".into(),
                            homeserver: "https://matrix-client.matrix.org/".into(),
                        },
                    },
                    current: false,
                },
            ],
            adding: false,
            matrix_login: None,
        };
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(
            serde_json::from_str::<ServersViewModel>(&json).unwrap(),
            view
        );
        for action in [
            Action::AddServer,
            Action::CancelAddServer,
            Action::SwitchServer { id: "b2".into() },
            Action::RenameServer {
                id: "b2".into(),
                name: "办公室".into(),
            },
            Action::RemoveServer { id: "b2".into() },
        ] {
            let json = serde_json::to_string(&action).unwrap();
            assert_eq!(serde_json::from_str::<Action>(&json).unwrap(), action);
        }
    }

    #[test]
    fn slot_kinds_round_trip_and_unknown_kinds_are_other() {
        for kind in [
            ServerKind::Wonton,
            ServerKind::Matrix {
                account: MatrixAccount {
                    user_id: "@alice:matrix.org".into(),
                    homeserver: "https://matrix-client.matrix.org/".into(),
                },
            },
            ServerKind::Other,
        ] {
            let json = serde_json::to_value(&kind).unwrap();
            assert_eq!(serde_json::from_value::<ServerKind>(json).unwrap(), kind);
        }
        assert_eq!(
            serde_json::from_str::<ServerKind>(r#"{"type":"future_slot","endpoint":"ignored"}"#)
                .unwrap(),
            ServerKind::Other
        );
    }
}
