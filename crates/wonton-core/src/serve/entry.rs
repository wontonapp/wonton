//! 入口、推送、隧道、监听和退出信号的接线。
use super::*;

pub(crate) async fn open_listener(
    listen: std::net::SocketAddr,
) -> anyhow::Result<wonton_core::entry_runtime::MainListener> {
    wonton_core::entry_runtime::MainListener::bind(listen)
        .await
        .with_context(|| format!("监听 {listen}"))
}

pub(crate) fn owner_token(cfg: &Config) -> anyhow::Result<String> {
    let path = cfg.core_state_dir().join("owner-token");
    let token = secret::load_or_create(&path)
        .with_context(|| format!("读取主人令牌 {}", path.display()))?;
    tracing::info!(path = %path.display(), "主人令牌已就绪");
    Ok(token)
}

pub(crate) fn open_domain(
    cfg: &Config,
    engine: &Engine,
    notices: &Option<Arc<NoticeRoom>>,
) -> anyhow::Result<Option<Arc<wonton_core::clients::domain::Domain>>> {
    let Some(dir) = &cfg.install_dir else {
        return Ok(None);
    };
    Ok(Some(wonton_core::clients::domain::Domain::new(
        dir.clone(),
        cfg.data_dir.clone(),
        engine.clone(),
        notices.clone(),
    )))
}

/// 用安装脚本装的核心（有安装目录）才管入口；核心不在容器里时由入口自己报 `core_not_in_container`。
/// 对账现在做一次：后面要解析入口地址的启动步骤等在这里。
pub(crate) async fn start_entry_runtime(
    cfg: &Config,
    engine: &Engine,
) -> Option<wonton_core::entry_containers::EntryContainers> {
    let dir = cfg.install_dir.as_ref()?;
    let containers = wonton_core::entry_containers::EntryContainers::new(
        engine.clone(),
        dir.clone(),
        cfg.data_dir.clone(),
        cfg.instance.clone(),
        cfg.core_container.clone(),
    );
    // 读不了设置（JSON 坏、解析器非法）是"不确定"：不按"无设置"删掉在跑的入口容器，交给运行循环
    // 带退避重试。Ok(None) 不代表不要入口：“不要名字”的安装本来就没有设置文件，对账按 App 里配的
    // 域名和证书决定。
    match wonton_core::entry_settings::EntrySettings::load(dir) {
        Err(error) => {
            tracing::warn!(%error, "入口设置读不了，现有入口容器不动，运行循环会带退避重试");
        }
        Ok(settings) => {
            if let Err(error) = containers.reconcile(settings.as_ref()).await {
                // 入口的循环会带退避重试，这里不挡住核心启动。
                tracing::warn!(category = error.category(), %error, "入口容器没对上");
            }
        }
    }
    Some(containers)
}

pub(crate) fn open_push(
    cfg: &Config,
    tunnel: &Tunnel,
    manager: &PluginManager,
) -> anyhow::Result<Push> {
    // 两个变量由配置层保证成对出现：设了才带 VAPID 头，发到 ntfy 这类地址也照样带，没有副作用。
    let vapid = match (&cfg.push_vapid_key, &cfg.push_vapid_subject) {
        (Some(key), Some(subject)) => Some(
            wonton_core::push::Vapid::from_key(key, subject.clone())
                .map_err(anyhow::Error::msg)
                .context("解析 WONTON_PUSH_VAPID_KEY")?,
        ),
        _ => None,
    };
    let path = cfg.core_state_dir().join("push.json");
    Push::open(&path, tunnel.devices.clone(), tunnel.endpoint.id(), vapid)
        .and_then(|push| push.with_proxy(manager.core_proxy()))
        .with_context(|| format!("打开推送登记表 {}", path.display()))
}

/// 帮助文档（ADR-0034）。版本号和升级接口报的是同一个。
pub(crate) async fn bind_tunnel(
    cfg: &Config,
    manager: &Arc<PluginManager>,
    members: Arc<Members>,
) -> anyhow::Result<Tunnel> {
    let devices = Devices::open(cfg.core_state_dir().join("devices.json"))?;
    members.recover_pending_devices(&devices)?;
    let key = tunnel::load_or_create_key(&cfg.core_state_dir().join("tunnel-key"))?;
    let settings = std::sync::Arc::new(
        wonton_core::relay_settings::RelaySettings::open(
            wonton_core::relay_settings::Startup::from_config(cfg),
        )
        .await
        .context("读中继表配置")?,
    );
    let (endpoint, fallback) = tunnel::bind(key, cfg.tunnel_port, &settings.table()).await?;
    settings.attach(endpoint.clone(), fallback);
    tracing::info!(
        endpoint_id = %endpoint.id(),
        sockets = ?endpoint.bound_sockets(),
        devices = devices.list().len(),
        "隧道入口已启动"
    );
    let addresses = manager.lan_addresses();
    let owner_languages: OwnerLanguages = {
        let manager = manager.clone();
        Arc::new(move || manager.owner_languages())
    };
    let tunnel = Tunnel::new(
        Arc::new(devices),
        members,
        owner_languages,
        endpoint,
        cfg.tunnel_port,
        settings,
        addresses.clone(),
    );
    tunnel::spawn_announcing(tunnel.endpoint.clone(), tunnel.port, addresses);
    Ok(tunnel)
}

pub(crate) async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("无法注册 SIGTERM 处理");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
    tracing::info!("收到退出信号");
}
