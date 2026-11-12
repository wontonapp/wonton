//! 启动接线的另一半：外部客户、好友记录、分享登记、监听与隧道入口、插件核对。
use super::*;

pub(super) async fn open_clients(
    cfg: &Config,
    engine: &Engine,
    _registries: &sharing_start::Registries,
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

/// Bot 的联系人（ADR-0045、ADR-0050）：要 Matrix 和服务器配对，没接 Matrix 的核心没有。
pub(super) fn open_contacts(
    cfg: &Config,
    (manager, members, owner): (&Arc<PluginManager>, &Arc<Members>, &Arc<OwnerStore>),
    push: Option<Arc<Push>>,
    matrix: Option<&Arc<Matrix>>,
    peers: Option<&Arc<peers::Peers>>,
) -> anyhow::Result<Option<Arc<wonton_core::contacts::Contacts>>> {
    let (Some(matrix), Some(peers)) = (matrix, peers) else {
        return Ok(None);
    };
    let wiring = wonton_core::contacts::wiring::Wiring {
        plugins: manager.clone(),
        members: members.clone(),
        owner: owner.clone(),
        push,
        matrix: matrix.clone(),
        peers: peers.clone(),
    };
    Ok(Some(wonton_core::contacts::wiring::start(
        &cfg.core_state_dir(),
        wiring,
    )?))
}

pub(super) async fn run_gateway(
    cfg: &Config,
    listener: wonton_core::entry_runtime::MainListener,
    tunnel: Tunnel,
    (peers, federation): (Option<Arc<peers::Peers>>, Option<FederationProtocol>),
    gateway: Arc<Gateway>,
    (manager, upgrades_after_serve): (Arc<PluginManager>, Arc<Upgrades>),
) -> anyhow::Result<()> {
    let push_listener = push_listener::bind(cfg.push_listen.as_deref()).await?;
    let matrix_rtc = push_listener::bind_matrix_rtc(
        cfg.matrix_rtc_listen.as_deref(),
        gateway.matrix.as_ref().map(|matrix| matrix.server_name()),
        manager.matrix_rtc_ca().as_deref(),
    )
    .await?;
    let app = gateway::router(gateway.clone());
    let public_entry = wonton_core::entry_runtime::bind().await?;
    let share_protocol = sharing_start::protocol(gateway.clone())?;
    let entry = spawn_tunnel(
        &tunnel,
        app.clone(),
        peers,
        federation,
        share_protocol,
        &manager,
    );
    tracing::info!(
        listen = %cfg.listen,
        url = %cfg.public.origin(cfg.public.core_host()),
        "网关已启动"
    );
    let served = tokio::spawn(push_listener::serve(
        listener,
        app,
        push_listener.map(|listener| (listener, gateway::push_router(gateway.clone()))),
        public_entry.map(|listener| {
            (
                listener,
                wonton_core::entry_runtime::router(gateway.clone()),
            )
        }),
        matrix_rtc.map(|listener| (listener, gateway::matrix_rtc_alias::router(gateway))),
    ));
    // 网关开始接了才核对插件：核对时渲染变了会重建容器，里面的一次性服务要经核心的地址访问核心。
    start_restored_plugins(cfg, &manager, upgrades_after_serve).await?;
    // 隧道跟网关同生命周期：网关结束（收到退出信号）后才关。放在核对之后、网关结束之前关，
    // 核心一启动完隧道就没了，手机配不上也连不回家。
    let served = served.await.context("网关任务退出");
    entry.shutdown().await;
    served?
}

/// 隧道入口：网关路由、联邦、服务器配对和分享处理器、插件端口解析器一起挂上。
fn spawn_tunnel(
    tunnel: &Tunnel,
    app: axum::Router,
    peers: Option<Arc<peers::Peers>>,
    federation: Option<FederationProtocol>,
    share: sharing_start::LimitedShares,
    manager: &Arc<PluginManager>,
) -> TunnelEntry {
    TunnelEntry::spawn_with_protocols(
        tunnel,
        app,
        extra_protocols(peers, federation, share),
        Arc::new(tunnel::PluginPortResolver::new(manager.clone())),
    )
}

pub(super) fn spawn_plugin_monitors(manager: &Arc<PluginManager>) {
    manager.spawn_lan_monitor();
    manager.spawn_egress_monitor();
    manager.spawn_upgrade_checks();
    manager.spawn_oom_watch();
    manager.spawn_container_watch();
    manager.spawn_mailpit_watch();
}
