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
    use crate::{
        Action, ConnectionCheck, ConnectionDetails, ConnectionNotice, ConnectionQuality, CoreError,
        DeviceInvite, DeviceNetwork, HomeAction, HomeFailure, HomePath, HomeStatus, HomeViewModel,
        PairingFailure, PairingStatus, ProxySettings, SecretString, SessionAction,
    };

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
        let action = Action::Session {
            action: SessionAction::SelectSession {
                session_id: "session-1".into(),
            },
        };
        let error = CoreError::InvalidInput {
            reason: "会话不存在".into(),
        };

        assert_eq!(round_trip(&view_model), view_model);
        assert_eq!(round_trip(&action), action);
        assert_eq!(round_trip(&error), error);
    }

    #[test]
    fn home_view_round_trips_and_redacts_secrets() {
        let home = HomeViewModel {
            device_id: "ab".repeat(32),
            server_id: Some("cd".repeat(32)),
            status: HomeStatus::Disconnected {
                reason: HomeFailure::Unauthorized,
                message: "服务端拒绝了本设备".into(),
            },
            path: Some(HomePath {
                relay: false,
                public_relay: false,
                lan: true,
                rtt_ms: 3,
            }),
            notice: Some(ConnectionNotice::ServerOffline { retrying: true }),
            details: ConnectionDetails {
                network: Some(DeviceNetwork::Wifi),
                relay: ConnectionCheck::Ok,
                server: ConnectionCheck::Failing,
                last_connected_at_ms: Some(1_793_778_000_000),
                handshake_ms: Some(120),
                disconnects: 2,
                failed_dials: 1,
                sent_packets: 900,
                lost_packets: 3,
                quality: ConnectionQuality::Poor,
                loss_per_mille: Some(73),
                rtt_ms: Some(1_240),
                last_failure: Some("timed out".into()),
                addresses_stale: true,
                udp_flows: Vec::new(),
                udp_buffer_evicted_datagrams: None,
            },
            proxy: Some(ProxySettings {
                host: "127.0.0.1".into(),
                port: 49152,
                username: "wonton".into(),
                password: "secret".into(),
                domain: "wonton.localhost".into(),
            }),
            web_url: Some("http://app-grafana.wonton.localhost:18480/".into()),
            pairing: Some(PairingStatus::Failed {
                reason: PairingFailure::Expired,
                message: "码过期了".into(),
            }),
            invite: Some(DeviceInvite::Ready {
                code: SecretString::new(
                    "wonton://pair?v=0&secret=00112233445566778899aabbccddeeff",
                ),
                expires_at_ms: 1_767_225_600_000,
            }),
            keep_connection: true,
            just_recovered: true,
        };
        assert_eq!(round_trip(&home), home);
        assert!(
            !format!("{home:?}").contains("secret"),
            "Debug 不能输出代理密码和给新设备的配对码"
        );
    }

    #[test]
    fn home_actions_round_trip_through_json() {
        for action in [
            Action::Home {
                action: HomeAction::ConnectHome {
                    ticket: "{}".into(),
                },
            },
            Action::Home {
                action: HomeAction::PairHome {
                    code: SecretString::new("wonton://pair?v=0"),
                    device_name: "phone".into(),
                },
            },
            Action::Home {
                action: HomeAction::CancelPairing,
            },
            Action::Home {
                action: HomeAction::InviteDevice,
            },
            Action::Home {
                action: HomeAction::AppForegrounded,
            },
            Action::Home {
                action: HomeAction::AppBackgrounded,
            },
            Action::Home {
                action: HomeAction::NetworkChanged {
                    network: DeviceNetwork::None,
                },
            },
            Action::Home {
                action: HomeAction::RetryConnection,
            },
        ] {
            assert_eq!(round_trip(&action), action);
        }
    }

    /// 配对码里有一次性的配对密钥：操作打进日志时只能看到打码后的样子。
    #[test]
    fn pairing_codes_are_redacted_in_debug_output() {
        let secret = "00112233445566778899aabbccddeeff";
        let action = Action::Home {
            action: HomeAction::PairHome {
                code: SecretString::new(format!("wonton://pair?v=0&secret={secret}")),
                device_name: "phone".into(),
            },
        };
        let printed = format!("{action:?}");
        assert!(!printed.contains(secret), "{printed}");
    }

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }
}
