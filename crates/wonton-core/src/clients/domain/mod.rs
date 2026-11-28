//! 自己的域名和它的证书（ADR-0089）：设置落盘、运行时签发、CAA 状态。
//!
//! 设置文件是 `entry/domain.json`（0600）：后缀、服务商、只写不读的凭据、订户协议的勾选和最近一次
//! 签发失败的错误。装机时写下的环境变量（`entry/settings.json` 和 `WONTON_ACME_*`）是初值，有设置
//! 文件以后以设置文件为准。“不要名字”的安装没有 `entry/settings.json`，也能只靠环境给 ACME 坐标。
//!
//! 凭据只在签发时写成 lego 的 dotenv（0600），只挂进 lego 容器，用完删掉。
//!
//! 按职责拆开：`access` 是构造和读后缀，`settings` 是设置文件，`acme` 是 ACME 坐标和凭据校验，
//! `issue` 是签发和续期，`view` 是设置页状态，`save` 是保存。

mod access;
mod acme;
mod issue;
mod save;
mod settings;
mod view;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

// 这几项只有测试和兄弟测试模块用，lib 里没人碰。
#[allow(unused_imports)]
use acme::DNS_ENV_FILE;
#[allow(unused_imports)]
pub(in crate::clients) use acme::{dotenv_names, dotenv_value};
pub use save::{PREVIOUS_SUFFIX_HEADER, SUFFIX_CHANGED_HEADER, Saved};
pub(in crate::clients) use settings::Store;
pub(crate) use settings::stored_suffix;
use tokio::sync::Mutex;
#[allow(unused_imports)]
pub(super) use view::now_ms;
use wonton_protocol::CaaState;

use crate::matrix::NoticeRoom;
use crate::plugin::docker::Engine;

/// 设置项的失败，接口按类别回 400 或 409。
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    /// 固定说法，值是 `gateway.ftl` 的 id，接口按 App 的语言写。
    #[error("{0}")]
    InvalidKey(&'static str),
    #[error("{0}")]
    ConflictKey(&'static str),
    /// 动态说法（读文件、解析外部返回值失败）。
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
}

/// 域名设置和签发。API 和续期循环共用一份。
pub struct Domain {
    install_dir: PathBuf,
    data_dir: PathBuf,
    pub(crate) engine: Engine,
    notices: Option<Arc<NoticeRoom>>,
    /// 串行化签发；同时来两次就排队，后一次用最新的设置。
    issuance: Mutex<()>,
    /// 已经排队的签发数；大于零时证书状态是 `issuing`。
    pub(crate) pending: AtomicUsize,
    /// 设置文件的读写锁，签发不占着它。
    writes: Mutex<()>,
    /// CAA 结果的短缓存（后缀加账号为键，60 秒）：设置页刷新别每次都等 DNS。
    caa_cache: Mutex<Option<(std::time::Instant, String, CaaState)>>,
    /// 新证书发布后叫醒续期循环：下一轮对账按新证书重写入口配置再重载。
    refresh: tokio::sync::Notify,
}

#[cfg(test)]
#[path = "../domain_tests/mod.rs"]
mod domain_tests;
