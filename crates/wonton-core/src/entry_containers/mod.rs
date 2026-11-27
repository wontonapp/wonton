//! 入口容器和连接器：核心按期望状态经 Docker API 建和删（ADR-0021 的“组件”）。
//!
//! 不写在主 compose 里：开关、换连接器、地址变化都要在运行时重建，compose 项目 `up` 过一次就不再变（ADR-0021
//! “入口容器、局域网转发、连接器都由核心管”）。网络仍由主 compose 渲染，核心只把容器接上去；这样 `WONTON_EXCLUDE_LISTEN`
//! 和 8484 的绑定在启动时就能定。
//!
//! 核心不在容器里（开发时没设 `WONTON_CORE_CONTAINER`）时没有 compose 给的那些网络，要入口就回
//! `core_not_in_container`，不降级、不绑 `0.0.0.0`。

use std::path::{Path, PathBuf};

use crate::clients::lan_forward::LanState;
use crate::plugin::docker::Engine;
use crate::plugin::names::Instance;
use wonton_protocol::{GateState, PublicSwitchState};

/// 容器创建参数：镜像、命令行、只读挂载、安全选项和端点。
mod create;
/// 错误类型和把内部错误归成入口错误。
mod error;
/// 局域网转发：两张网络、容器和它的 HAProxy 配置。
mod lan_forward;
/// 网络接入：compose 项目网络、固定地址和别名。
mod network;
mod options;
/// 按目标对账：写配置、建删容器、必要时让入口重读。
mod reconcile;
mod reload;
/// 删除与重建：拆栈时删两个容器；对账时缺的建、参数变了删了重建。
mod remove;
/// 期望状态：要服务哪些名字、入口和连接器要不要。
mod target;

pub use error::EntryError;
pub use reload::reload_haproxy;

/// 组件的标签值，也是镜像 `wonton.core.components` 里声明的名字。
pub const ENTRY_COMPONENT: &str = "entry";
pub const CONNECTOR_COMPONENT: &str = "connector";
pub const LAN_FORWARD_COMPONENT: &str = "lan-forward";

pub const ENTRY_EDGE_ALIAS: &str = "entry-edge.wonton.internal";
pub const ENTRY_CLIENTS_ALIAS: &str = "entry.wonton.internal";
pub const CONNECTOR_ALIAS: &str = "connector.wonton.internal";
/// 入口在 `lanfwd` 上的名字：局域网前端绑在它上面（[`crate::entry_haproxy::LAN_BIND`]）。
pub const ENTRY_LANFWD_ALIAS: &str = "entry-lanfwd.wonton.internal";

/// 入口 HAProxy 主进程的管理地址：只有核心和入口在 `clients` 网络上，这个端口不出那个网络。
pub const ENTRY_RELOAD_ADDR: &str = "entry.wonton.internal:8404";
/// 连接器的管理地址（连接器在 `relay` 和 `edge` 上）。
pub const CONNECTOR_RELOAD_ADDR: &str = "connector.wonton.internal:8404";

/// 连接器把公网 TCP 转发接进入口的端口（路由器和自建 frps 都用）。
const CONNECTOR_PORT: u32 = 8443;

/// 局域网转发在宿主上发布的默认端口（`WONTON_CLIENT_PORT`）。
pub const DEFAULT_CLIENT_PORT: u16 = 8483;

#[derive(Clone)]
pub struct EntryContainers {
    engine: Engine,
    install_dir: PathBuf,
    data_dir: PathBuf,
    instance: Instance,
    core_container: Option<String>,
    /// 局域网入口用哪个地址、`*.lan` 记录对不对，由局域网检测更新（[`crate::clients::lan_forward`]）。
    lan: std::sync::Arc<std::sync::Mutex<LanState>>,
    /// 局域网转发在宿主上发布的端口。
    client_port: u16,
    /// 连接器发布的地址和端口；空时看 `WONTON_ENTRY_BIND`/`WONTON_ENTRY_PORT`。
    binding: Option<(String, String)>,
    operation: std::sync::Arc<tokio::sync::Mutex<Option<crate::entry_haproxy::Model>>>,
    /// 入口主进程的管理地址；测试指向临时端口。
    reload_addr: String,
    /// 上一次成功让入口读过的 `site.pem` 摘要：同名续期换的只是证书内容，配置文本不变，也要重载。
    loaded_cert: std::sync::Arc<std::sync::Mutex<Option<u64>>>,
    /// 上一轮该重载但没成：配置或证书都可能已经不再变化，必须跨轮把这笔账留住。
    reload_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 周期对账和一次开关下发不交叉，免得两边各按一半旧状态建删。
    reconcile: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl EntryContainers {
    pub fn new(
        engine: Engine,
        install_dir: PathBuf,
        data_dir: PathBuf,
        instance: Instance,
        core_container: Option<String>,
    ) -> Self {
        Self {
            engine,
            install_dir,
            data_dir,
            instance,
            core_container,
            lan: std::sync::Arc::new(std::sync::Mutex::new(LanState::Unknown)),
            client_port: DEFAULT_CLIENT_PORT,
            binding: None,
            operation: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            reload_addr: ENTRY_RELOAD_ADDR.to_owned(),
            loaded_cert: std::sync::Arc::new(std::sync::Mutex::new(None)),
            reload_pending: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            reconcile: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// 测试用：固定发布地址和端口，不读进程环境。
    pub fn with_binding(mut self, bind: &str, port: &str) -> Self {
        self.binding = Some((bind.to_owned(), port.to_owned()));
        self
    }
    #[must_use]
    pub fn with_client_port(mut self, port: u16) -> Self {
        self.client_port = port;
        self
    }

    /// 测试用：把入口的主进程管理端口指到临时监听。
    #[must_use]
    pub fn with_reload_addr(mut self, addr: String) -> Self {
        self.reload_addr = addr;
        self
    }

    pub fn lan(&self) -> LanState {
        self.lan.lock().expect("局域网入口状态锁").clone()
    }

    pub(crate) fn set_lan(&self, state: LanState) {
        *self.lan.lock().expect("局域网入口状态锁") = state;
    }

    pub fn client_port(&self) -> u16 {
        self.client_port
    }

    pub fn core_in_container(&self) -> bool {
        self.core_container.is_some()
    }

    pub fn engine(&self) -> Engine {
        self.engine.clone()
    }

    pub fn install_dir(&self) -> &Path {
        &self.install_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 公网证书列表是不是空的。文件不在也是空：没有入口容器的安装本来就没有这份列表。
    pub fn public_crt_list_empty(&self) -> std::io::Result<bool> {
        let path = self.install_dir.join("entry/certs/public.crt-list");
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(text.trim().is_empty()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error),
        }
    }

    /// 连接器容器在不在跑。
    pub async fn connector_running(&self) -> Result<bool, EntryError> {
        self.running(CONNECTOR_COMPONENT).await
    }

    /// 某个组件的容器在不在跑。
    async fn running(&self, component: &str) -> Result<bool, EntryError> {
        Ok(self
            .engine
            .inspect_container(&self.name(component))
            .await?
            .and_then(|info| info.state)
            .and_then(|state| state.running)
            == Some(true))
    }

    /// 容器和证书列表都做不成时（设置读不了、Docker 出错）的两道闸门结果；核心那道闸门只是代码，照旧完成。
    pub fn report_switch(&self, detail: &str) -> PublicSwitchState {
        PublicSwitchState {
            connector: GateState::Failed {
                detail: detail.to_owned(),
            },
            crt_list: GateState::Failed {
                detail: detail.to_owned(),
            },
            core: GateState::Done,
        }
    }
    /// 容器的名字：和插件资源一样带实例名，几套核心共用一台 Docker 时不会撞。
    fn name(&self, component: &str) -> String {
        match self.instance.name() {
            None => format!("wonton-{component}"),
            Some(instance) => format!("wonton-{instance}--{component}"),
        }
    }
}

#[cfg(test)]
mod tests;
