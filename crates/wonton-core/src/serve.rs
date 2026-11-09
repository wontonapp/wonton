//! 启动接线的另一半：监听与隧道入口、插件核对。
use super::*;

pub(super) async fn serve_listeners(
    cfg: &Config,
    listener: wonton_core::entry_runtime::MainListener,
    gateway: Arc<Gateway>,
    tunnel: Tunnel,
    (peers, federation): (Option<Arc<peers::Peers>>, Option<FederationProtocol>),
    (manager, upgrades_after_serve): (Arc<PluginManager>, Arc<Upgrades>),
) -> anyhow::Result<()> {
    let push_listener = push_listener::bind(cfg.push_listen.as_deref()).await?;
    let app = gateway::router(gateway.clone());
    let public_entry = wonton_core::entry_runtime::bind().await?;
    let entry =
        TunnelEntry::spawn_with_protocols(&tunnel, app.clone(), extra_protocols(peers, federation));
    tracing::info!(
        listen = %cfg.listen,
        url = %cfg.public.origin(cfg.public.core_host()),
        "网关已启动"
    );
    let served = tokio::spawn(push_listener::serve(
        listener,
        app,
        push_listener.map(|listener| (listener, gateway::push_router(gateway.clone()))),
        public_entry.map(|listener| (listener, wonton_core::entry_runtime::router(gateway))),
    ));
    // 网关开始接了才核对插件：核对时渲染变了会重建容器，里面的一次性服务要经核心的地址访问核心。
    start_restored_plugins(cfg, &manager, upgrades_after_serve).await?;
    // 隧道跟网关同生命周期：网关结束（收到退出信号）后才关。放在核对之后、网关结束之前关，
    // 核心一启动完隧道就没了，手机配不上也连不回家。
    let served = served.await.context("网关任务退出");
    entry.shutdown().await;
    served?
}

pub(super) fn spawn_plugin_monitors(manager: &Arc<PluginManager>) {
    manager.spawn_lan_monitor();
    manager.spawn_egress_monitor();
    manager.spawn_upgrade_checks();
    manager.spawn_oom_watch();
    manager.spawn_container_watch();
}
