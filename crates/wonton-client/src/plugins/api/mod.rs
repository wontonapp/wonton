//! 核心的插件管理接口（ADR-0017 的“管理接口”）：请求走本地代理，核心凭隧道身份认出这台已配对设备。请求不带
//! `Origin` 和 `Sec-Fetch-Site`，核心据此知道它不是网页发起的。

mod client;
mod error;
mod response;
mod version;

pub(crate) use client::{CoreApi, INSTALL_TIMEOUT, READ_TIMEOUT, SOURCE_TIMEOUT};
pub(super) use client::{not_connected, not_paired, proxied_client};
pub(crate) use error::failure_kind;
pub(crate) use response::{CoreApiError, to_core_error};
pub(crate) use version::ServerVersion;
