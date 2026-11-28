//! 核心的插件管理接口（ADR-0017 的“管理接口”）：请求走本地代理，核心凭隧道身份认出这台已配对设备。请求不带
//! `Origin` 和 `Sec-Fetch-Site`，核心据此知道它不是网页发起的。

mod client;
mod error;
mod response;

pub(crate) use client::{CoreApi, INSTALL_TIMEOUT, READ_TIMEOUT, SOURCE_TIMEOUT};
pub(super) use client::{not_connected, not_paired, proxied_client};
pub(crate) use error::failure_kind;
pub(crate) use response::{CoreApiError, to_core_error};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Method;
use serde::Deserialize;

use crate::plugins::lock;

/// 问服务器版本：接口已经回过 404，等久了不如先按原来的类别报。
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// 服务器版本的一次性缓存，随作用域活着：升级状态的每次成功回应都写进来。
#[derive(Clone, Default)]
pub(crate) struct ServerVersion(Arc<VersionCache>);

#[derive(Default)]
struct VersionCache {
    value: Mutex<Option<String>>,
    /// 问过一次了：服务器不报版本时不再每次都问。
    asked: AtomicBool,
}

impl ServerVersion {
    /// 记下服务器自己报的版本；空的不覆盖已有的。
    pub(super) fn seed(&self, version: Option<String>) {
        if let Some(version) = version {
            *lock(&self.0.value) = Some(version);
        }
    }

    fn known(&self) -> Option<String> {
        lock(&self.0.value).clone()
    }

    /// 还不知道时问服务器一次（核心升级状态的 `current.version`）；问不到就是不知道。
    async fn get(&self, api: &CoreApi) -> Option<String> {
        if let Some(version) = self.known() {
            return Some(version);
        }
        if self.0.asked.swap(true, Ordering::SeqCst) {
            return None;
        }
        #[derive(Deserialize)]
        struct Reported {
            current: Current,
        }
        #[derive(Deserialize)]
        struct Current {
            #[serde(default)]
            version: Option<String>,
        }
        let probed = match api
            .exchange(
                Method::GET,
                api.url(&["core", "upgrade"], &[]),
                None,
                "application/json",
                VERSION_TIMEOUT,
            )
            .await
        {
            Ok((status, bytes)) if status < 400 => serde_json::from_slice::<Reported>(&bytes)
                .ok()
                .and_then(|reported| reported.current.version),
            _ => None,
        };
        self.seed(probed.clone());
        probed
    }
}
