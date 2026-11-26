//! 假首屏：一台“家里”的服务器、已连接的家、以及添加设备页走查用的假配对码。

use std::time::{SystemTime, UNIX_EPOCH};

use wonton_protocol::{
    DeviceInvite, DeviceNetwork, HomePath, HomeStatus, HomeViewModel, SecretString, ServerKind,
    ServerName, ServerRow, ServersViewModel,
};

/// 添加设备页走查用的假配对码：格式同真码（`wonton://pair?v=0&…`），指向 example.org 和 192.0.2.10 的假地址。
const FAKE_PAIRING_CODE: &str = "wonton://pair?v=0&id=9f2ce1a4b7d04e5f8a3c6d1b0e9f27a4&relays=https%3A%2F%2Frelay.example.org%3A443&addr=192.0.2.10%3A3478&domain=wonton.example.org&core=https%3A%2F%2Fwonton.example.org%2F&secret=u7Xk2pQ9wR4tY6uI8oP1aS3dF5gH7jK9lZ2xC4vB6n8"; // gitleaks:allow：写死的假码，不是凭据

pub(super) fn fake_home(mut home: HomeViewModel) -> HomeViewModel {
    home.server_id = Some("ui-fake".into());
    home.status = HomeStatus::Connected;
    home.path = Some(HomePath {
        relay: false,
        public_relay: false,
        lan: true,
        rtt_ms: 12,
        relay_url: None,
    });
    home.details.network = Some(DeviceNetwork::Wifi);
    home.details.relay = Default::default();
    // 添加设备页走查：摆一个十分钟内不过期的假码。假数据下 dispatch 忽略 Home 动作（见 dispatch.rs），
    // 码不会被换掉或用掉。
    home.invite = Some(DeviceInvite::Ready {
        code: SecretString::new(FAKE_PAIRING_CODE),
        expires_at_ms: (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            + 600_000) as u64,
    });
    home
}

pub(super) fn fake_servers() -> ServersViewModel {
    ServersViewModel {
        servers: vec![ServerRow {
            id: "ui-fake".into(),
            name: ServerName::Named {
                name: "Home".into(),
            },
            kind: ServerKind::Wonton,
            current: true,
        }],
        adding: false,
        matrix_login: None,
    }
}
