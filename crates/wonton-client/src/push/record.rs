//! 这个槽位上次登记给服务器的推送地址。推送地址变了时，不是当前这台的服务器靠它判断要不要临时连上去重新登记
//! （ADR-0042）。只记地址和公钥，认证密钥不落盘。

use std::path::Path;

use serde::{Deserialize, Serialize};
use wonton_protocol::PushEndpoint;

const FILE: &str = "push-endpoint.json";

#[derive(PartialEq, Eq, Serialize, Deserialize)]
struct Recorded {
    url: String,
    p256dh: String,
}

impl Recorded {
    fn of(endpoint: &PushEndpoint) -> Self {
        Self {
            url: endpoint.url.clone(),
            p256dh: endpoint.p256dh.clone(),
        }
    }
}

/// 记不下来只影响下次推送地址变了时要不要重新登记这一台，不影响这次登记，所以只记日志。
pub(crate) fn save(dir: &Path, endpoint: &PushEndpoint) {
    let saved = serde_json::to_vec(&Recorded::of(endpoint))
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(dir.join(FILE), json));
    if let Err(error) = saved {
        tracing::warn!(%error, "could not save the registered push endpoint");
    }
}

pub(crate) fn clear(dir: &Path) {
    match std::fs::remove_file(dir.join(FILE)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            tracing::warn!(%error, "could not delete the registered push endpoint");
        }
        _ => {}
    }
}

/// 这个槽位登记过的就是这个地址。没记过、读不出来都算不是。
pub(crate) fn matches(dir: &Path, endpoint: &PushEndpoint) -> bool {
    std::fs::read(dir.join(FILE))
        .ok()
        .and_then(|json| serde_json::from_slice::<Recorded>(&json).ok())
        .is_some_and(|recorded| recorded == Recorded::of(endpoint))
}
