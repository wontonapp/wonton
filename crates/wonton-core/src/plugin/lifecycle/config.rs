//! 插件管理器需要的那部分核心配置。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::super::names::Instance;
use crate::config::PublicUrls;

/// 插件管理器需要的那部分核心配置。
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    pub public: PublicUrls,
    pub data_dir: PathBuf,
    pub apps_dir: PathBuf,
    pub shares_dir: PathBuf,
    /// 宿主网络中继的 unix 套接字目录（[`super::super::config::DEFAULT_RELAY_DIR`]）。
    pub relay_dir: PathBuf,
    pub core_container: Option<String>,
    pub tz: String,
    pub puid: u32,
    pub pgid: u32,
    pub install_timeout: Duration,
    /// 主人的用户名，也是主人的 Matrix 用户名（localpart）。
    pub owner: String,
    /// 主人的可变资料；插件邮箱由固定用户名派生，不取真实邮箱（ADR-0056）。
    pub profile: Arc<crate::owner::OwnerStore>,
    /// 核心网关监听的端口：插件经核心在插件网络里的别名（如 `matrix.wonton.internal`）访问它。
    pub gateway_port: u16,
    /// 核心给插件容器设置的 systemd slice；空值表示不设置父 cgroup。
    pub plugin_slice: Option<String>,
    /// 核心的实例名（`WONTON_INSTANCE`），决定插件的资源名和标签。
    pub instance: Instance,
}
