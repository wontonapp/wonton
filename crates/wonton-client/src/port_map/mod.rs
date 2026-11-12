//! 插件端口在本机同号映射（ADR-0072）：只监听 `127.0.0.1`，TCP 一条本机连接一条 `Port` 流，UDP 每个来源地址
//! 一个流向。屏幕上显示着插件的页面时自动映射（`SetVisiblePlugin`），页面看不见、App 进后台就停，回前台恢复，
//! 手动开关不受影响；主人的开关按服务器存在槽位目录里。绑定和解绑都在一个后台任务里按顺序做，ViewModel 只在做完
//! 之后发出：看到“关”时端口已经空出来了。映射状态每次变化打一行 INFO 日志：端口、结果、触发原因，只在变化时打。

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
    PortMappingWarning, PortProtocol,
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
    /// 屏幕上正显示着页面的插件（`SetVisiblePlugin`），进后台留着，页面换走才清。
    visible: Option<String>,
    /// App 在前台。进后台自动映射失效，回前台恢复，壳不用重报。
    foreground: bool,
    /// 已安装列表里声明了 `tunnel.ports` 的插件。
    declared: BTreeMap<String, Vec<MappedPort>>,
    connected: bool,
    /// 唤醒这次 reconcile 的事由（手动开关、页面可见、前后台、隧道、安装列表），取走打进生命周期日志。
    triggers: BTreeSet<&'static str>,
    /// 当前路径走的是 n0 公共中继：带宽不够传画面，开着的映射要如实提示（ADR-0072）。
    public_relay: bool,
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
                foreground: true,
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
        shared.triggers.insert("manual");
        drop(shared);
        self.wake.notify_one();
        Ok(())
    }

    /// `SetVisiblePlugin`：屏幕上换了页面。看不见的插件只要手动开关还开着就继续映射。
    pub(crate) fn set_visible(&self, plugin_id: Option<String>) {
        let mut shared = lock(&self.shared);
        if shared.visible == plugin_id {
            return;
        }
        shared.visible = plugin_id;
        shared.triggers.insert("visible");
        drop(shared);
        self.wake.notify_one();
    }

    /// `AppForegrounded`/`AppBackgrounded`：自动映射只在 App 看得见时算数，页面的可见性留着。
    pub(crate) fn set_foreground(&self, foreground: bool) {
        let mut shared = lock(&self.shared);
        if shared.foreground == foreground {
            return;
        }
        shared.foreground = foreground;
        shared.triggers.insert(if foreground {
            "foreground"
        } else {
            "background"
        });
        drop(shared);
        self.wake.notify_one();
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
            shared.triggers.insert("install-list");
            drop(shared);
            self.wake.notify_one();
        }
    }

    fn set_connected(&self, connected: bool) {
        let mut shared = lock(&self.shared);
        if shared.connected != connected {
            shared.connected = connected;
            shared.triggers.insert("tunnel");
            drop(shared);
            self.wake.notify_one();
        }
    }

    fn set_public_relay(&self, public_relay: bool) {
        let mut shared = lock(&self.shared);
        if shared.public_relay != public_relay {
            shared.public_relay = public_relay;
            drop(shared);
            self.wake.notify_one();
        }
    }

    /// 让本机监听和开关、声明一致，再发出 ViewModel。只在后台任务里调，同一时刻只有一次。
    async fn reconcile(
        &self,
        active: &mut HashMap<String, Active>,
        reconnected: bool,
        logged: &mut Option<Logged>,
    ) {
        // 触发原因和要映射的集合取自同一次加锁：不会漏掉刚发生的开关变化，也不会拿旧原因对上新状态。
        let (wanted, reason) = {
            let mut shared = lock(&self.shared);
            let triggers = std::mem::take(&mut shared.triggers);
            let joined = triggers.into_iter().collect::<Vec<_>>().join(",");
            let reason = (!joined.is_empty()).then_some(joined);
            let wanted: BTreeMap<String, Vec<MappedPort>> = shared
                .declared
                .iter()
                .filter(|(id, _)| maps(&shared, id))
                .map(|(id, ports)| (id.clone(), ports.clone()))
                .collect();
            (wanted, reason)
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
        self.publish(active, reason.as_deref(), logged);
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

    fn publish(
        &self,
        active: &HashMap<String, Active>,
        reason: Option<&str>,
        logged: &mut Option<Logged>,
    ) {
        let shared = lock(&self.shared);
        let public_relay = shared.public_relay;
        let mappings: Vec<PluginPortMapping> = shared
            .declared
            .iter()
            .map(|(id, ports)| {
                let enabled = shared.enabled.contains(id);
                let state = state(maps(&shared, id), active.get(id), shared.connected);
                PluginPortMapping {
                    plugin_id: id.clone(),
                    enabled,
                    state,
                    ports: ports.clone(),
                    warning: warning(state, public_relay),
                }
            })
            .collect();
        self.view.send_if_modified(|view| {
            let changed = view.port_mappings != mappings;
            view.port_mappings = mappings;
            changed
        });
        log_changes(&shared, active, reason, logged);
    }
}

impl Drop for PortMapper {
    /// 后台任务在等唤醒；叫醒它，它发现映射器没了就放掉所有端口。
    fn drop(&mut self) {
        self.wake.notify_one();
    }
}

/// 这个插件要不要映射：主人手动打开了，或者 App 看得见它的页面。手动映射进后台照常。
fn maps(shared: &Shared, plugin: &str) -> bool {
    shared.enabled.contains(plugin)
        || (shared.foreground && shared.visible.as_deref() == Some(plugin))
}

fn state(mapped: bool, active: Option<&Active>, connected: bool) -> PortMappingState {
    let Some(active) = active.filter(|_| mapped) else {
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

/// 映射真的开着（端口绑上了、没有拒绝原因）而路径又在 n0 公共中继时才提示；别的状态各有各的话，
/// 不叠这一条。
fn warning(state: PortMappingState, public_relay: bool) -> Option<PortMappingWarning> {
    (state == PortMappingState::On && public_relay)
        .then_some(PortMappingWarning::PublicRelayBandwidth)
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
    let mut logged: Option<Logged> = None;
    loop {
        let Some(link) = mapper.upgrade() else {
            break;
        };
        let now_connected = lock(&link.shared).connected;
        // 重新连上的可能是升级过的服务器：“太旧”要重新判断。
        link.reconcile(&mut active, now_connected && !connected, &mut logged)
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
        let (connected, public_relay) = {
            let view = home.borrow_and_update();
            let connected = view.status == HomeStatus::Connected;
            // 没连上就没有路径；路径换了中继（掉档回退）也要跟着改。
            (
                connected,
                connected && view.path.as_ref().is_some_and(|path| path.public_relay),
            )
        };
        let Some(link) = mapper.upgrade() else {
            return;
        };
        link.set_connected(connected);
        link.set_public_relay(public_relay);
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

/// 上次发布过的每个插件的状态和端口串，生命周期日志拿它对变化。
type Logged = HashMap<String, (PortMappingState, String)>;

/// 生命周期日志：发布后和上次发布对一遍，变了才打一行 INFO——排障时看映射什么时候绑上、因为什么没了。
/// 首次发布只记账：开机就开着的映射不是变化。打开端口被服务器拒绝这类原因没有壳动作触发，reason 留空。
fn log_changes(
    shared: &Shared,
    active: &HashMap<String, Active>,
    reason: Option<&str>,
    logged: &mut Option<Logged>,
) {
    let mut ids: BTreeSet<String> = shared.declared.keys().cloned().collect();
    ids.extend(active.keys().cloned());
    ids.extend(logged.iter().flatten().map(|(id, _)| id.clone()));
    let now: Vec<(String, PortMappingState, String)> = ids
        .into_iter()
        .map(|id| {
            let ports = shared
                .declared
                .get(&id)
                .map(|ports| ports_str(ports))
                .unwrap_or_default();
            let state = state(maps(shared, &id), active.get(&id), shared.connected);
            (id, state, ports)
        })
        .collect();
    let Some(seen) = logged else {
        *logged = Some(
            now.into_iter()
                .map(|(id, state, ports)| (id, (state, ports)))
                .collect(),
        );
        return;
    };
    for (id, now, ports) in now {
        let changed = match seen.get(&id) {
            None => now != PortMappingState::Off,
            Some((was, was_ports)) => {
                *was != now || (now != PortMappingState::Off && *was_ports != ports)
            }
        };
        if changed {
            match reason {
                Some(reason) => {
                    tracing::info!(plugin = %id, ports = %ports, state = ?now, reason = %reason, "port mapping changed")
                }
                None => {
                    tracing::info!(plugin = %id, ports = %ports, state = ?now, "port mapping changed")
                }
            }
        }
        seen.insert(id, (now, ports));
    }
}

/// 日志里的端口串：`8000/tcp 3478/udp`。
fn ports_str(ports: &[MappedPort]) -> String {
    ports
        .iter()
        .map(|mapped| {
            let protocol = match mapped.protocol {
                PortProtocol::Tcp => "tcp",
                PortProtocol::Udp => "udp",
                PortProtocol::Other => "other",
            };
            format!("{}/{}", mapped.port, protocol)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_on_mapping_over_the_public_relay_carries_the_warning() {
        let on = PortMappingState::On;
        assert_eq!(
            warning(on, true),
            Some(PortMappingWarning::PublicRelayBandwidth)
        );
        assert_eq!(warning(on, false), None);
        // 自建中继照传，不提示；没开着的映射也没有。
        for state in [
            PortMappingState::Off,
            PortMappingState::PortInUse,
            PortMappingState::TunnelDisconnected,
            PortMappingState::ServerTooOld,
            PortMappingState::Other,
        ] {
            assert_eq!(warning(state, true), None);
            assert_eq!(warning(state, false), None);
        }
    }
}
