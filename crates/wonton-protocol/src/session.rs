use serde::{Deserialize, Serialize};

/// 会话列表页的 ViewModel：壳拿到后直接渲染，不再做任何推导。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SessionListViewModel {
    /// 按显示顺序排列的会话，最新的在前。
    pub sessions: Vec<SessionRow>,
}

/// 会话列表里的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SessionRow {
    /// 会话的稳定标识，操作里用它引用会话。
    pub id: String,
    pub title: String,
    pub is_selected: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, CoreError};

    /// 契约：ViewModel、操作和错误都能经 JSON 往返，供将来的 Web 端经 WebSocket 收发。
    #[test]
    fn interface_types_round_trip_through_json() {
        let view_model = SessionListViewModel {
            sessions: vec![SessionRow {
                id: "session-1".into(),
                title: "家里的 NAS".into(),
                is_selected: true,
            }],
        };
        let action = Action::SelectSession {
            session_id: "session-1".into(),
        };
        let error = CoreError::InvalidInput {
            reason: "会话不存在".into(),
        };

        assert_eq!(round_trip(&view_model), view_model);
        assert_eq!(round_trip(&action), action);
        assert_eq!(round_trip(&error), error);
    }

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }
}
