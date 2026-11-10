//! 启动接线的另一半：外部客户、好友记录、监听与隧道入口、插件核对。
use super::*;

pub(super) async fn open_clients(
    cfg: &Config,
    engine: &Engine,
) -> anyhow::Result<(
    HttpClient,
    HttpClient,
    Option<(Hydra, String)>,
    Option<Arc<Matrix>>,
)> {
    let http = HttpClient::new();
    let system_http =
        HttpClient::for_system_services(engine, cfg.core_container.as_deref()).await?;
    let hydra = connect_hydra(cfg, &system_http).await?;
    let matrix = connect_matrix(cfg, &system_http).await?;
    Ok((http, system_http, hydra, matrix))
}

pub(super) fn open_friends(
    cfg: &Config,
) -> anyhow::Result<(Arc<Friends>, Option<Arc<FederationBlocks>>)> {
    let friends = Arc::new(Friends::open(cfg.core_state_dir().join("friends.json"))?);
    let blocks = match (&cfg.install_dir, &cfg.core_container) {
        (Some(_), Some(_)) => Some(Arc::new(FederationBlocks::open(
            cfg.core_state_dir().join("federation-blocks.json"),
        )?)),
        _ => None,
    };
    Ok((friends, blocks))
}

pub(super) async fn run_gateway(
    cfg: &Config,
    listener: TcpListener,
    tunnel: Tunnel,
    (peers, federation): (Option<Arc<peers::Peers>>, Option<FederationProtocol>),
    gateway: Arc<Gateway>,
    (manager, upgrades_after_serve): (Arc<PluginManager>, Arc<Upgrades>),
) -> anyhow::Result<()> {
    let push_listener = push_listener::bind(cfg.push_listen.as_deref()).await?;
    let app = gateway::router(gateway.clone());
    let entry = spawn_tunnel(&tunnel, app.clone(), peers, federation, &manager);
    tracing::info!(
        listen = %cfg.listen,
        url = %cfg.public.origin(cfg.public.core_host()),
        "网关已启动"
    );
    let served = tokio::spawn(push_listener::serve(
        listener,
        app,
        push_listener.map(|listener| (listener, gateway::push_router(gateway))),
    ));
    // 网关开始接了才核对插件：核对时渲染变了会重建容器，里面的一次性服务要经核心的地址访问核心。
    start_restored_plugins(cfg, &manager, upgrades_after_serve).await?;
    // 隧道跟网关同生命周期：网关结束（收到退出信号）后才关。放在核对之后、网关结束之前关，
    // 核心一启动完隧道就没了，手机配不上也连不回家。
    let served = served.await.context("网关任务退出");
    entry.shutdown().await;
    served?
}

/// 隧道入口：网关路由、联邦和服务器配对处理器、插件端口解析器一起挂上。
fn spawn_tunnel(
    tunnel: &Tunnel,
    app: axum::Router,
    peers: Option<Arc<peers::Peers>>,
    federation: Option<FederationProtocol>,
    manager: &Arc<PluginManager>,
) -> TunnelEntry {
    TunnelEntry::spawn_with_protocols(
        tunnel,
        app,
        extra_protocols(peers, federation),
        Arc::new(tunnel::PluginPortResolver::new(manager.clone())),
    )
}

pub(super) fn spawn_plugin_monitors(manager: &Arc<PluginManager>) {
    manager.spawn_lan_monitor();
    manager.spawn_egress_monitor();
    manager.spawn_upgrade_checks();
    manager.spawn_oom_watch();
    manager.spawn_container_watch();
}
