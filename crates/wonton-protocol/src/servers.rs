//! 配过的服务器：同一时间连一台，其他几台的数据各自留在手机上，每台都推送（ADR-0042）。

use serde::{Deserialize, Serialize};

/// 配过的服务器和正在连的是哪一台。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ServersViewModel {
    /// 配好了的服务器，按添加的顺序。
    pub servers: Vec<ServerRow>,
    /// 正在添加一台：当前连的是一个还没配对的新位置，壳显示配对页，配对页上可以
    /// [`Action::CancelAddServer`](crate::Action::CancelAddServer) 回到原来那台。
    pub adding: bool,
}

/// 一台配过的服务器。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ServerRow {
    /// 这台服务器在本机的 ID。操作和通知都用它引用服务器，和服务器自己的 endpoint ID 不是一回事。
    pub id: String,
    /// 主人起的名字；没改过时是“家里的服务器”“服务器 2”这样的默认名。
    pub name: String,
    /// 正在连的就是这台。
    pub current: bool,
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
                    name: "家里的服务器".into(),
                    current: true,
                },
                ServerRow {
                    id: "b2".into(),
                    name: "办公室".into(),
                    current: false,
                },
            ],
            adding: false,
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
}
