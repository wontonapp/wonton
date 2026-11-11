//! 插件端口在本机同号映射（ADR-0072）：只监听 `127.0.0.1`，TCP 一条本机连接一条 `Port` 流，UDP 每个来源地址
//! 一个流向。主人的开关按服务器存在槽位目录里；绑定和解绑都在一个后台任务里按顺序做，ViewModel 只在做完之后
//! 发出：看到“关”时端口已经空出来了。

mod listen;
mod store;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, HomeStatus, MappedPort, PluginPortMapping, PluginsViewModel, PortMappingState,
    PortProtocol,
};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use listen::Ctx;

/// 一个插件的绑定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Bound,
    /// 有一个端口被别的程序占着：一个也不绑，不换号。
    PortInUse,
    /// 不是“被占用”的绑定失败（日志里有原因）。
    BindFailed,
}

#[derive(Debug, Default)]
struct Shared {
    enabled: BTreeSet<String>,
    /// 已安装列表里声明了 `tunnel.ports` 的插件。
    declared: BTreeMap<String, Vec<MappedPort>>,
    connected: bool,
}

/// 后台任务手里的一个插件：监听任务和它们报回来的拒绝原因。
struct Active {
    ports: Vec<MappedPort>,
    outcome: Outcome,
    refusal: Arc<Mutex<Option<PortMappingState>>>,
    tasks: Vec<JoinHandle<()>>,
}

pub(crate) struct PortMapper {
    home: Arc<HomeLink>,
    view: watch::Sender<PluginsViewModel>,
    file: PathBuf,
    shared: Mutex<Shared>,
    wake: Arc<Notify>,
}

impl PortMapper {
    pub(crate) fn new(
        runtime: &Handle,
        home: Arc<HomeLink>,
        dir: &Path,
        view: watch::Sender<PluginsViewModel>,
    ) -> Arc<Self> {
        let file = dir.join(store::FILE);
        let mapper = Arc::new(Self {
            shared: Mutex::new(Shared {
                enabled: store::load(&file),
                ..Shared::default()
            }),
            home,
            view,
            file,
            wake: Arc::new(Notify::new()),
        });
        runtime.spawn(reconcile_loop(Arc::downgrade(&mapper), mapper.wake.clone()));
        runtime.spawn(follow_tunnel(Arc::downgrade(&mapper)));
        runtime.spawn(follow_installed(
            Arc::downgrade(&mapper),
            mapper.view.subscribe(),
        ));
        mapper
    }

    /// `SetPortMapping`：先存开关，存不下就不改；绑定在后台做，结果进 `PluginsViewModel.port_mappings`。
    pub(crate) fn set_enabled(&self, plugin_id: String, enabled: bool) -> Result<(), CoreError> {
        let mut shared = lock(&self.shared);
        let mut next = shared.enabled.clone();
        if enabled {
            next.insert(plugin_id);
        } else {
            next.remove(&plugin_id);
        }
        store::save(&self.file, &next).map_err(|err| CoreError::Storage {
            reason: l10n!(
                CATALOG,
                language(),
                "home-port-mapping-save-failed",
                error = err.to_string()
            ),
        })?;
        shared.enabled = next;
        drop(shared);
        self.wake.notify_one();
        Ok(())
    }

    /// 已安装列表变了：按里面的 `tunnel_ports` 重新对一遍。
    fn set_declared(&self, declared: BTreeMap<String, Vec<MappedPort>>) {
        let declared: BTreeMap<_, _> = declared
            .into_iter()
            .filter(|(_, ports)| !ports.is_empty())
            .collect();
        let mut shared = lock(&self.shared);
        if shared.declared != declared {
            shared.declared = declared;
            drop(shared);
            self.wake.notify_one();
        }
    }

    fn set_connected(&self, connected: bool) {
        let mut shared = lock(&self.shared);
        if shared.connected != connected {
            shared.connected = connected;
            drop(shared);
            self.wake.notify_one();
        }
    }

    /// 让本机监听和开关、声明一致，再发出 ViewModel。只在后台任务里调，同一时刻只有一次。
    async fn reconcile(&self, active: &mut HashMap<String, Active>, reconnected: bool) {
        let wanted: BTreeMap<String, Vec<MappedPort>> = {
            let shared = lock(&self.shared);
            shared
                .declared
                .iter()
                .filter(|(id, _)| shared.enabled.contains(*id))
                .map(|(id, ports)| (id.clone(), ports.clone()))
                .collect()
        };
        let stale: Vec<String> = active
            .iter()
            .filter(|(id, entry)| {
                wanted.get(*id) != Some(&entry.ports) || entry.outcome != Outcome::Bound
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(entry) = active.remove(&id) {
                stop(entry).await;
            }
        }
        for (id, ports) in wanted {
            match active.get(&id) {
                Some(entry) if reconnected => *lock(&entry.refusal) = None,
                Some(_) => {}
                None => {
                    let entry = self.start(&id, ports);
                    active.insert(id, entry);
                }
            }
        }
        self.publish(active);
    }

    fn start(&self, plugin: &str, ports: Vec<MappedPort>) -> Active {
        let refusal = Arc::new(Mutex::new(None));
        let (outcome, tasks) = match bind_all(&ports) {
            Ok(sockets) => {
                let ctx = Ctx {
                    home: self.home.clone(),
                    plugin: plugin.into(),
                    refusal: refusal.clone(),
                    wake: self.wake.clone(),
                };
                (Outcome::Bound, listen::spawn(sockets, &ctx))
            }
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => (Outcome::PortInUse, Vec::new()),
            Err(err) => {
                tracing::error!(plugin, "failed to listen on a mapped port: {err}");
                (Outcome::BindFailed, Vec::new())
            }
        };
        Active {
            ports,
            outcome,
            refusal,
            tasks,
        }
    }

    fn publish(&self, active: &HashMap<String, Active>) {
        let mappings: Vec<PluginPortMapping> = {
            let shared = lock(&self.shared);
            shared
                .declared
                .iter()
                .map(|(id, ports)| {
                    let enabled = shared.enabled.contains(id);
                    PluginPortMapping {
                        plugin_id: id.clone(),
                        enabled,
                        state: state(enabled, active.get(id), shared.connected),
                        ports: ports.clone(),
                    }
                })
                .collect()
        };
        self.view.send_if_modified(|view| {
            let changed = view.port_mappings != mappings;
            view.port_mappings = mappings;
            changed
        });
    }
}

impl Drop for PortMapper {
    /// 后台任务在等唤醒；叫醒它，它发现映射器没了就放掉所有端口。
    fn drop(&mut self) {
        self.wake.notify_one();
    }
}

fn state(enabled: bool, active: Option<&Active>, connected: bool) -> PortMappingState {
    let Some(active) = active.filter(|_| enabled) else {
        return PortMappingState::Off;
    };
    match active.outcome {
        Outcome::PortInUse => PortMappingState::PortInUse,
        Outcome::BindFailed => PortMappingState::Other,
        // 断开时上一次的拒绝已经过时；重新连上后清掉，再开时重新判断。
        Outcome::Bound if !connected => PortMappingState::TunnelDisconnected,
        Outcome::Bound => lock(&active.refusal).unwrap_or(PortMappingState::On),
    }
}

/// 一个插件的端口全绑上才算数；有一个绑不上就全放掉，不留半个映射。协议未知的端口不绑。
fn bind_all(ports: &[MappedPort]) -> io::Result<Vec<listen::Socket>> {
    let mut sockets = Vec::new();
    for mapped in ports {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, mapped.port));
        let socket = match mapped.protocol {
            PortProtocol::Tcp => listen::Socket::Tcp(std::net::TcpListener::bind(addr)?),
            PortProtocol::Udp => listen::Socket::Udp(std::net::UdpSocket::bind(addr)?),
            PortProtocol::Other => continue,
        };
        sockets.push(socket);
    }
    Ok(sockets)
}

/// 停掉监听任务并等它们真的结束：套接字随任务一起关，等完了端口才空出来。
async fn stop(entry: Active) {
    for task in &entry.tasks {
        task.abort();
    }
    for task in entry.tasks {
        task.await.ok();
    }
}

async fn reconcile_loop(mapper: Weak<PortMapper>, wake: Arc<Notify>) {
    let mut active = HashMap::new();
    let mut connected = false;
    loop {
        let Some(link) = mapper.upgrade() else {
            break;
        };
        let now_connected = lock(&link.shared).connected;
        // 重新连上的可能是升级过的服务器：“太旧”要重新判断。
        link.reconcile(&mut active, now_connected && !connected)
            .await;
        connected = now_connected;
        drop(link);
        wake.notified().await;
    }
    for (_, entry) in active.drain() {
        stop(entry).await;
    }
}

async fn follow_tunnel(mapper: Weak<PortMapper>) {
    let Some(mut home) = mapper.upgrade().map(|mapper| mapper.home.subscribe()) else {
        return;
    };
    loop {
        let connected = home.borrow_and_update().status == HomeStatus::Connected;
        let Some(link) = mapper.upgrade() else {
            return;
        };
        link.set_connected(connected);
        drop(link);
        if home.changed().await.is_err() {
            return;
        }
    }
}

/// 声明跟着已安装列表走；列表还没取到（或者换了服务器被清掉）时一个也不映射。自己写的 `port_mappings` 也会叫醒
/// 这里，声明没变就什么也不做。
async fn follow_installed(mapper: Weak<PortMapper>, mut view: watch::Receiver<PluginsViewModel>) {
    loop {
        let declared: BTreeMap<String, Vec<MappedPort>> = view
            .borrow_and_update()
            .installed
            .iter()
            .flat_map(|installed| &installed.plugins)
            .map(|plugin| (plugin.id.clone(), plugin.tunnel_ports.clone()))
            .collect();
        let Some(link) = mapper.upgrade() else {
            return;
        };
        link.set_declared(declared);
        drop(link);
        if view.changed().await.is_err() {
            return;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
