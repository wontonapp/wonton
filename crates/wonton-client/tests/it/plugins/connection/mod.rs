//! 连不上、被拒、重试：连不上由 `HomeViewModel` 说，不进插件页；换服务器时不提前报“已连接”。

use std::time::{Duration, Instant};

use wonton_client::{ConnectionNotice, HomeFailure, HomeStatus};

use super::rig::Rig;

mod dial;
mod reject;

/// 建连尝试可能要等到握手超时。
const DIAL: Duration = Duration::from_secs(90);

/// 等第一次建连失败，返回隧道报的原因。
async fn first_failure(rig: &Rig) -> HomeFailure {
    let deadline = Instant::now() + DIAL;
    loop {
        if let HomeStatus::Disconnected { reason, .. } = rig.app.home().status {
            return reason;
        }
        assert!(Instant::now() < deadline, "{:?}", rig.app.home());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// 服务器拒绝了这台设备，重连也没用，状态不该再回到“连接中”。
async fn stays_rejected(rig: &Rig) {
    assert_eq!(first_failure(rig).await, HomeFailure::Unauthorized);
    assert_eq!(
        rig.app.home().notice,
        Some(ConnectionNotice::DeviceRejected)
    );
    let quiet = Instant::now() + Duration::from_secs(8);
    while Instant::now() < quiet {
        let status = rig.app.home().status;
        assert!(
            matches!(status, HomeStatus::Disconnected { .. }),
            "被拒绝以后又去重连了：{status:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
